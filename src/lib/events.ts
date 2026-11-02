/**
 * Finds things a message is scheduling: calendar invitations (.ics), schema.org events in
 * HTML (tickets, reservations), and plain-text dates with a time ("Tuesday at 14:00",
 * "3. 11. o 9:30", "23 грудня о 18:00"). Also picks out places to show on a map.
 * Deliberately conservative: a date alone (no time) is only used from structured data.
 */

export interface EventSuggestion {
  title: string;
  start: Date;
  end?: Date;
  allDay: boolean;
  location?: string;
  source: "invite" | "schema" | "text";
}

export type RsvpKind = "yes" | "maybe" | "no";

export interface Detected {
  events: EventSuggestion[];
  places: string[];
  /** Video call link (Meet, Zoom, Teams, …), if the message has one. */
  join?: { url: string; service: string };
  /** The invitation's own Yes/Maybe/No links (Google Calendar and similar). */
  rsvp: { kind: RsvpKind; url: string }[];
}

const MEETING_LINK =
  /https:\/\/(?:meet\.google\.com\/[a-z]{3}-[a-z]{4}-[a-z]{3}|[\w.-]*zoom\.us\/(?:j|my|w)\/[^\s<>"')]+|teams\.microsoft\.com\/l\/meetup-join\/[^\s<>"')]+|teams\.live\.com\/meet\/[^\s<>"')]+|[\w.-]+\.webex\.com\/[^\s<>"')]+|whereby\.com\/[^\s<>"')]+|meet\.jit\.si\/[^\s<>"')]+)/i;

const SERVICES: [RegExp, string][] = [
  [/meet\.google\.com/i, "Google Meet"],
  [/zoom\.us/i, "Zoom"],
  [/teams\.(microsoft|live)\.com/i, "Teams"],
  [/webex\.com/i, "Webex"],
  [/whereby\.com/i, "Whereby"],
  [/meet\.jit\.si/i, "Jitsi"],
];

const RSVP_WORDS: Record<RsvpKind, RegExp> = {
  yes: /^(yes|accept|áno|ano|так|ja|oui|sí|tak)$/i,
  maybe: /^(maybe|tentative|možno|možná|можливо|vielleicht|peut-être|quizás|może)$/i,
  no: /^(no|decline|nie|ne|ні|nein|non)$/i,
};

function findJoin(...sources: (string | null | undefined)[]): Detected["join"] {
  for (const src of sources) {
    const url = src?.match(MEETING_LINK)?.[0];
    if (url) {
      const service = SERVICES.find(([re]) => re.test(url))?.[1] ?? "";
      return { url: url.replace(/&amp;/g, "&"), service };
    }
  }
  return undefined;
}

/** Anchors whose whole text is Yes / Maybe / No, as in calendar invitations. */
function findRsvp(html: string): Detected["rsvp"] {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const out: Detected["rsvp"] = [];
  doc.querySelectorAll<HTMLAnchorElement>("a[href]").forEach((a) => {
    const text = (a.textContent ?? "").trim();
    const href = a.getAttribute("href") ?? "";
    if (!/^https:\/\//i.test(href)) return;
    for (const kind of ["yes", "maybe", "no"] as RsvpKind[]) {
      if (RSVP_WORDS[kind].test(text) && !out.some((r) => r.kind === kind)) {
        out.push({ kind, url: href });
      }
    }
  });
  // Only an actual set of choices counts; a lone "No" link is something else.
  return out.length >= 2
    ? out.sort(
        (x, y) => ["yes", "maybe", "no"].indexOf(x.kind) - ["yes", "maybe", "no"].indexOf(y.kind),
      )
    : [];
}

const IS_URL = /^\s*(https?:\/\/|www\.)/i;

const MONTHS: Record<string, number> = {};
const MONTH_NAMES: string[][] = [
  ["jan", "january", "januar", "január", "januára", "leden", "ledna", "січня", "січень"],
  ["feb", "february", "február", "februára", "únor", "února", "лютого", "лютий"],
  ["mar", "march", "marec", "marca", "březen", "března", "березня", "березень"],
  ["apr", "april", "apríl", "apríla", "duben", "dubna", "квітня", "квітень"],
  ["may", "máj", "mája", "květen", "května", "травня", "травень"],
  ["jun", "june", "jún", "júna", "červen", "června", "червня", "червень"],
  ["jul", "july", "júl", "júla", "červenec", "července", "липня", "липень"],
  ["aug", "august", "augusta", "srpen", "srpna", "серпня", "серпень"],
  ["sep", "sept", "september", "septembra", "září", "вересня", "вересень"],
  ["oct", "october", "október", "októbra", "říjen", "října", "жовтня", "жовтень"],
  ["nov", "november", "novembra", "listopad", "listopadu", "листопада", "листопад"],
  ["dec", "december", "decembra", "prosinec", "prosince", "грудня", "грудень"],
];
MONTH_NAMES.forEach((names, i) => names.forEach((n) => (MONTHS[n] = i)));

/** 0 = Sunday, like Date.getDay(). */
const WEEKDAYS: Record<string, number> = {};
[
  ["sunday", "sun", "nedeľa", "nedeľu", "neděle", "neděli", "неділя", "неділю"],
  ["monday", "mon", "pondelok", "pondělí", "понеділок"],
  ["tuesday", "tue", "utorok", "úterý", "вівторок"],
  ["wednesday", "wed", "streda", "stredu", "středa", "středu", "середа", "середу"],
  ["thursday", "thu", "štvrtok", "čtvrtek", "четвер"],
  ["friday", "fri", "piatok", "pátek", "пʼятниця", "п'ятниця", "п'ятницю", "пʼятницю"],
  ["saturday", "sat", "sobota", "sobotu", "субота", "суботу"],
].forEach((names, i) => names.forEach((n) => (WEEKDAYS[n] = i)));

const RELATIVE: Record<string, number> = {
  today: 0,
  dnes: 0,
  сьогодні: 0,
  tomorrow: 1,
  zajtra: 1,
  zítra: 1,
  завтра: 1,
};

const L = "\\p{L}";
const MONTH_ALT = Object.keys(MONTHS)
  .sort((a, b) => b.length - a.length)
  .join("|");
const WEEKDAY_ALT = Object.keys(WEEKDAYS)
  .sort((a, b) => b.length - a.length)
  .join("|");
const RELATIVE_ALT = Object.keys(RELATIVE).join("|");

/** 14:00, 14.00 h, 2pm, 2:30 pm, 9 h */
const TIME = new RegExp(
  `(?<![\\d.:])([01]?\\d|2[0-3])(?:[:.]([0-5]\\d))?\\s*(am|pm|a\\.m\\.|p\\.m\\.|h|hod\\.?|год\\.?)?(?![\\d${L}])`,
  "giu",
);

const DATE_PATTERNS: { re: RegExp; read: (m: RegExpExecArray) => DatePart | null }[] = [
  // 3. 11. 2026, 03.11.2026, 3.11., 3/11/2026
  {
    re: /(?<![\d.])(\d{1,2})\s?[./]\s?(\d{1,2})\s?[./](?:\s?(\d{4}|\d{2}))?(?![\d])/gu,
    read: (m) => ({ day: +m[1], month: +m[2] - 1, year: m[3] ? normYear(+m[3]) : undefined }),
  },
  // 2026-11-03
  {
    re: /(?<!\d)(\d{4})-(\d{2})-(\d{2})(?!\d)/gu,
    read: (m) => ({ year: +m[1], month: +m[2] - 1, day: +m[3] }),
  },
  // 23 December, 23. decembra, 23 грудня
  {
    re: new RegExp(`(?<!\\d)(\\d{1,2})\\.?\\s+(${MONTH_ALT})\\.?(?:\\s+(\\d{4}))?(?!${L})`, "giu"),
    read: (m) => ({
      day: +m[1],
      month: MONTHS[m[2].toLowerCase()],
      year: m[3] ? +m[3] : undefined,
    }),
  },
  // December 23, Dec 23rd 2026
  {
    re: new RegExp(
      `(?<!${L})(${MONTH_ALT})\\.?\\s+(\\d{1,2})(?:st|nd|rd|th)?(?:,?\\s+(\\d{4}))?(?!\\d)`,
      "giu",
    ),
    read: (m) => ({
      day: +m[2],
      month: MONTHS[m[1].toLowerCase()],
      year: m[3] ? +m[3] : undefined,
    }),
  },
  // (on|v|у|в) Tuesday
  {
    re: new RegExp(`(?<!${L})(${WEEKDAY_ALT})(?!${L})`, "giu"),
    read: (m) => ({ weekday: WEEKDAYS[m[1].toLowerCase()] }),
  },
  // tomorrow, zajtra, завтра
  {
    re: new RegExp(`(?<!${L})(${RELATIVE_ALT})(?!${L})`, "giu"),
    read: (m) => ({ offset: RELATIVE[m[1].toLowerCase()] }),
  },
];

interface DatePart {
  day?: number;
  month?: number;
  year?: number;
  weekday?: number;
  offset?: number;
}

function normYear(y: number): number {
  return y < 100 ? 2000 + y : y;
}

/** Resolves a date relative to when the message was sent; undefined if impossible. */
function resolveDate(p: DatePart, sent: Date): Date | undefined {
  const base = new Date(sent.getFullYear(), sent.getMonth(), sent.getDate());
  if (p.offset !== undefined) {
    base.setDate(base.getDate() + p.offset);
    return base;
  }
  if (p.weekday !== undefined) {
    const diff = (p.weekday - base.getDay() + 7) % 7 || 7;
    base.setDate(base.getDate() + diff);
    return base;
  }
  if (p.day === undefined || p.month === undefined || p.month < 0 || p.month > 11) return undefined;
  if (p.day < 1 || p.day > 31) return undefined;
  let year = p.year ?? base.getFullYear();
  let d = new Date(year, p.month, p.day);
  if (d.getMonth() !== p.month) return undefined;
  // "23 December" written in late December of last year means next year's.
  if (p.year === undefined && d.getTime() < base.getTime() - 7 * 86_400_000) {
    year += 1;
    d = new Date(year, p.month, p.day);
  }
  return d;
}

function readTime(m: RegExpExecArray): { h: number; min: number } | null {
  let h = +m[1];
  const min = m[2] ? +m[2] : 0;
  const suffix = (m[3] ?? "").toLowerCase().replace(/\./g, "");
  if (!m[2] && !suffix) return null; // a bare number is not a time
  if (suffix === "pm" && h < 12) h += 12;
  if (suffix === "am" && h === 12) h = 0;
  return { h, min };
}

/** Plain-text events: a date-like phrase with a time within a short distance. */
export function textEvents(text: string, title: string, sent: Date, now: Date): EventSuggestion[] {
  const out: EventSuggestion[] = [];
  const times = [...text.matchAll(TIME)]
    .map((m) => ({ index: m.index ?? 0, end: (m.index ?? 0) + m[0].length, t: readTime(m) }))
    .filter(
      (x): x is { index: number; end: number; t: { h: number; min: number } } => x.t !== null,
    );
  if (times.length === 0) return out;

  for (const { re, read } of DATE_PATTERNS) {
    for (const m of text.matchAll(re)) {
      const part = read(m);
      const day = part && resolveDate(part, sent);
      if (!day) continue;
      const at = m.index ?? 0;
      const near = times.find(
        (t) => Math.abs(t.index - (at + m[0].length)) <= 40 || Math.abs(at - t.end) <= 40,
      );
      if (!near) continue;
      const start = new Date(day);
      start.setHours(near.t.h, near.t.min, 0, 0);
      if (start.getTime() < now.getTime() - 3600_000) continue; // already over
      if (out.some((e) => e.start.getTime() === start.getTime())) continue;
      out.push({ title, start, allDay: false, source: "text" });
    }
  }
  return out.slice(0, 3);
}

// --- iCalendar (invitations) ---

function unfold(ics: string): string[] {
  return ics.replace(/\r?\n[ \t]/g, "").split(/\r?\n/);
}

function icsUnescape(v: string): string {
  return v.replace(/\\n/gi, "\n").replace(/\\([,;\\])/g, "$1");
}

/** DTSTART value → Date. UTC ("…Z") is exact; floating/TZID times are read as local time. */
function icsDate(value: string, params: string): { date: Date; allDay: boolean } | undefined {
  const m = value.match(/^(\d{4})(\d{2})(\d{2})(?:T(\d{2})(\d{2})(\d{2})?(Z)?)?$/);
  if (!m) return undefined;
  const [, y, mo, d, h, mi, s, z] = m;
  if (!h || /VALUE=DATE(?!-)/i.test(params))
    return { date: new Date(+y, +mo - 1, +d), allDay: true };
  const date = z
    ? new Date(Date.UTC(+y, +mo - 1, +d, +h, +mi, +(s ?? 0)))
    : new Date(+y, +mo - 1, +d, +h, +mi, +(s ?? 0));
  return { date, allDay: false };
}

export function icsEvents(ics: string, fallbackTitle: string): EventSuggestion[] {
  const events: EventSuggestion[] = [];
  let cur: Partial<EventSuggestion> | null = null;
  for (const line of unfold(ics)) {
    if (line === "BEGIN:VEVENT") cur = {};
    else if (line === "END:VEVENT" && cur) {
      if (cur.start) {
        events.push({
          title: cur.title || fallbackTitle,
          start: cur.start,
          end: cur.end,
          allDay: Boolean(cur.allDay),
          location: cur.location,
          source: "invite",
        });
      }
      cur = null;
    } else if (cur) {
      const idx = line.indexOf(":");
      if (idx < 0) continue;
      const [name, ...params] = line.slice(0, idx).split(";");
      const value = line.slice(idx + 1);
      const p = params.join(";");
      if (name === "SUMMARY") cur.title = icsUnescape(value);
      else if (name === "LOCATION") cur.location = icsUnescape(value) || undefined;
      else if (name === "DTSTART") {
        const r = icsDate(value, p);
        if (r) {
          cur.start = r.date;
          cur.allDay = r.allDay;
        }
      } else if (name === "DTEND") cur.end = icsDate(value, p)?.date;
    }
  }
  return events;
}

// --- schema.org JSON-LD in HTML (tickets, reservations) ---

type Json = Record<string, unknown>;

function asArray(v: unknown): Json[] {
  if (Array.isArray(v)) return v.flatMap(asArray);
  if (v && typeof v === "object") {
    const o = v as Json;
    return "@graph" in o ? asArray(o["@graph"]) : [o];
  }
  return [];
}

function placeText(v: unknown): string | undefined {
  if (!v) return undefined;
  if (typeof v === "string") return v;
  const o = v as Json;
  const addr = o.address as Json | string | undefined;
  const parts = [
    o.name,
    typeof addr === "string"
      ? addr
      : addr && [addr.streetAddress, addr.addressLocality].filter(Boolean).join(", "),
  ].filter(Boolean);
  return parts.length ? parts.join(", ") : undefined;
}

export function schemaEvents(html: string, fallbackTitle: string): EventSuggestion[] {
  const out: EventSuggestion[] = [];
  const blocks = html.matchAll(/<script[^>]+application\/ld\+json[^>]*>([\s\S]*?)<\/script>/gi);
  for (const [, json] of blocks) {
    let data: unknown;
    try {
      data = JSON.parse(json);
    } catch {
      continue;
    }
    for (const item of asArray(data)) {
      const type = String(item["@type"] ?? "");
      const target = (item.reservationFor as Json | undefined) ?? item;
      const startRaw = (target.startDate ?? target.departureTime ?? item.checkinTime) as
        string | undefined;
      if (!/Event|Reservation/i.test(type) || !startRaw) continue;
      const start = new Date(startRaw);
      if (Number.isNaN(start.getTime())) continue;
      const endRaw = (target.endDate ?? target.arrivalTime ?? item.checkoutTime) as
        string | undefined;
      out.push({
        title: String(target.name ?? item.name ?? fallbackTitle),
        start,
        end: endRaw ? new Date(endRaw) : undefined,
        allDay: !/T\d/.test(startRaw),
        location: placeText(target.location ?? target.departureStation ?? target.address),
        source: "schema",
      });
    }
  }
  return out;
}

// --- places ---

const PLACE_LABEL =
  /(?<!\p{L})(?:location|where|address|venue|place|miesto|adresa|místo|kde|місце|адреса|де|место)\s*:\s*([^\n]{3,120})/giu;
const MAP_LINK =
  /https?:\/\/(?:www\.)?(?:google\.[a-z.]+\/maps|maps\.google\.[a-z.]+|goo\.gl\/maps|maps\.app\.goo\.gl|maps\.apple\.com)[^\s<>"')]*/gi;

function placeFromMapLink(url: string): string | undefined {
  try {
    const u = new URL(url);
    const q =
      u.searchParams.get("q") ?? u.searchParams.get("query") ?? u.searchParams.get("address");
    if (q) return q;
    const m = u.pathname.match(/\/maps\/(?:place|search)\/([^/@]+)/);
    return m ? decodeURIComponent(m[1].replace(/\+/g, " ")) : undefined;
  } catch {
    return undefined;
  }
}

export function textPlaces(text: string): string[] {
  const out = new Set<string>();
  for (const m of text.matchAll(PLACE_LABEL)) out.add(m[1].trim().replace(/[.;,]$/, ""));
  for (const m of text.matchAll(MAP_LINK)) {
    const p = placeFromMapLink(m[0]);
    if (p) out.add(p);
  }
  return [...out];
}

export function detect(
  msg: { subject: string; body: string; html?: string | null; ics?: string | null; sentAt: string },
  now = new Date(),
): Detected {
  const title =
    msg.subject.replace(/^\s*((re|fwd?|aw|wg|odp|sv|vs)\s*:\s*)+/i, "").trim() || "Event";
  const sent = new Date(msg.sentAt);
  const structured = [
    ...(msg.ics ? icsEvents(msg.ics, title) : []),
    ...(msg.html ? schemaEvents(msg.html, title) : []),
  ];
  const events = structured.length ? structured : textEvents(msg.body, title, sent, now);
  const join = findJoin(...events.map((e) => e.location), msg.ics, msg.html, msg.body);
  const places = new Set<string>(
    events.map((e) => e.location).filter((l): l is string => Boolean(l)),
  );
  textPlaces(msg.body).forEach((p) => places.add(p));
  if (msg.html) {
    // Labels live in the text; map links usually sit in href attributes.
    const text = msg.html.replace(/<[^>]+>/g, "\n");
    const links = (msg.html.match(MAP_LINK) ?? []).join("\n");
    textPlaces(`${text}\n${links}`).forEach((p) => places.add(p));
  }
  return {
    events: events
      // A meeting link is not a place; keep it off the calendar entry's location too.
      .map((e) => (e.location && IS_URL.test(e.location) ? { ...e, location: undefined } : e))
      .filter((e) => e.start.getTime() > now.getTime() - 86_400_000),
    places: [...places].filter((p) => !IS_URL.test(p)).slice(0, 3),
    join,
    rsvp: msg.html ? findRsvp(msg.html) : [],
  };
}

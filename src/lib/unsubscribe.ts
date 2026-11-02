import type { Message } from "../types";

/**
 * How to leave the mailing list a message came from. The `List-Unsubscribe` header is the
 * same in every language; without it (older cached mail, senders that skip it) the footer
 * link is found by its text in many languages, or by its address.
 */
export type UnsubscribeAction =
  /** RFC 8058: one POST unsubscribes, no page to visit. */
  | { kind: "oneClick"; url: string }
  /** A page where the user confirms. */
  | { kind: "web"; url: string }
  /** A prepared email to the list server. */
  | { kind: "mail"; url: string };

/** Link text that means "unsubscribe". Substrings, so no \b (it is ASCII-only in JS). */
const UNSUB_TEXT = new RegExp(
  [
    // English
    "unsubscribe",
    "unsubcribe",
    "opt[- ]?out",
    // Slovak, Czech
    "odhlásiť",
    "odhlásenie",
    "zrušiť odber",
    "odhlásit",
    "odhlášení",
    "zrušit odběr",
    // Ukrainian, Russian
    "відписатис[яь]",
    "відписка",
    "відмовитис[яь] від розсилки",
    "скасувати підписку",
    "отписаться",
    "отписка",
    "отказаться от рассылки",
    // German, Dutch
    "abmelden",
    "abbestellen",
    "austragen",
    "abmeldung",
    "uitschrijven",
    "afmelden",
    // French, Spanish, Italian, Portuguese, Romanian
    "désabonner",
    "désinscri",
    "désabonnement",
    "desinscri",
    "darse de baja",
    "dar(?:se|me|te)? de baja",
    "date de baja",
    "cancelar (?:la )?suscripción",
    "anular (?:la )?suscripción",
    "disiscriv",
    "cancella(?:re)? (?:l')?iscrizione",
    "annulla(?:re)? (?:l')?iscrizione",
    "descadastr",
    "cancelar (?:a )?(?:inscrição|subscrição)",
    "dezabona",
    // Polish, Hungarian, Croatian/Serbian/Slovenian
    "wypisz",
    "wypisać",
    "rezygnacja z subskrypcji",
    "zrezygnuj z subskrypcji",
    "leiratkoz",
    "odjav",
    // Nordic
    "avregistrera",
    "avsluta prenumeration",
    "afmeld",
    "frameld",
    "meld (?:deg )?av",
    "peruuta tilaus",
    // Turkish, Greek
    "abonelikten (?:çık|ayrıl)",
    "διαγραφ",
    // Chinese, Japanese, Korean
    "退订",
    "取消订阅",
    "退訂",
    "取消訂閱",
    "配信停止",
    "購読解除",
    "配信解除",
    "수신\\s?거부",
    "구독\\s?취소",
  ].join("|"),
  "iu",
);

/** Addresses of unsubscribe pages, whatever the link says. */
const UNSUB_URL =
  /un-?subscribe|[/?&._-]unsub(?:[/?&=._-]|$)|opt[-_]?out|abmeld|abbestell|d[eé]sinscri|desabonn|uitschrijv|afmeld|odhlas/i;

const ANCHOR = /<a\b([^>]*)>([\s\S]*?)<\/a\s*>/gi;
const HREF = /\bhref\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/i;
const WEB_URL = /https?:\/\/[^\s<>"')\]]+/gi;
/** How far before a URL in plain text its label may start ("To unsubscribe, click: …"). */
const LABEL_SPAN = 80;

function decodeEntities(s: string): string {
  return s
    .replace(/&nbsp;/gi, " ")
    .replace(/&#(\d+);/g, (_, n: string) => String.fromCodePoint(Number(n)))
    .replace(/&#x([0-9a-f]+);/gi, (_, n: string) => String.fromCodePoint(parseInt(n, 16)))
    .replace(/&quot;/gi, '"')
    .replace(/&#39;|&apos;/gi, "'")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&amp;/gi, "&");
}

const isWeb = (url: string) => /^https?:\/\//i.test(url.trim());

/** The footer link: by its text first (most reliable), then by its address. */
export function findHtmlLink(html: string): string | null {
  // Conditional comments hold Outlook-only copies of the layout.
  const clean = html.replace(/<!--[\s\S]*?-->/g, "");
  let byUrl: string | null = null;
  for (const [, attrs, inner] of clean.matchAll(ANCHOR)) {
    const m = HREF.exec(attrs);
    const href = m ? decodeEntities((m[1] ?? m[2] ?? m[3] ?? "").trim()) : "";
    if (!isWeb(href)) continue;
    const text = decodeEntities(inner.replace(/<[^>]*>/g, " "))
      .replace(/\s+/g, " ")
      .trim();
    if (UNSUB_TEXT.test(text)) return href;
    if (!byUrl && UNSUB_URL.test(href)) byUrl = href;
  }
  return byUrl;
}

/** In plain text: a URL labelled as unsubscribe just before it, or an unsubscribe URL. */
export function findTextLink(text: string): string | null {
  let byUrl: string | null = null;
  for (const m of text.matchAll(WEB_URL)) {
    const url = m[0].replace(/[.,;:!?]+$/, "");
    const label = text.slice(Math.max(0, m.index - LABEL_SPAN), m.index);
    // Only the label's own line or the one before it, not an earlier paragraph.
    const near = label.split(/\n\s*\n/).pop() ?? "";
    if (UNSUB_TEXT.test(near)) return url;
    if (!byUrl && UNSUB_URL.test(url)) byUrl = url;
  }
  return byUrl;
}

/**
 * `links`: also look for a footer link. Only for mail a machine sent, so a person's message
 * that happens to mention opting out of something never offers to unsubscribe.
 */
export function unsubscribeAction(
  m: Pick<Message, "outgoing" | "unsubscribe" | "html" | "body" | "quoted">,
  { links }: { links: boolean },
): UnsubscribeAction | null {
  if (m.outgoing) return null;
  const header = m.unsubscribe;
  if (header?.url && header.oneClick) return { kind: "oneClick", url: header.url };
  if (header?.url && isWeb(header.url)) return { kind: "web", url: header.url };
  const link = !links
    ? null
    : (m.html && findHtmlLink(m.html)) || findTextLink(`${m.body}\n${m.quoted ?? ""}`);
  if (link) return { kind: "web", url: link };
  if (header?.mailto) return { kind: "mail", url: header.mailto };
  return null;
}

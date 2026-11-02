import DOMPurify from "dompurify";

/**
 * Turns a raw HTML email into a self-contained document for a sandboxed iframe:
 * sanitised (no scripts, forms, frames), quoted replies optionally removed, and a
 * Content-Security-Policy that blocks every remote load (tracking pixels, web fonts,
 * CSS backgrounds) unless the user allowed it.
 */

export type RemotePolicy =
  { kind: "none" } | { kind: "all" } | { kind: "hosts"; domains: string[] };

export interface BubbleColors {
  text: string;
  link: string;
}

/**
 * How a message is shown: `bubble` (readable, inside the chat bubble), `card` (readable, a
 * wider card for newsletter-style layouts), `original` (the sender's design on white).
 */
export type Display = "bubble" | "card" | "original";

export interface PreparedHtml {
  srcdoc: string;
  display: Display;
  /** Newsletter-style layout (tables, images, backgrounds): shown as a white card. */
  rich: boolean;
  /** Domains remote content would come from (images, fonts, CSS). */
  remoteDomains: string[];
  /** Quoted reply/forward parts were found (and removed if asked). */
  hasQuote: boolean;
  sizeHints: SizeHints;
}

/** What a frame will hold, to reserve its space until the document has laid itself out. */
export interface SizeHints {
  /** Characters in each run of text between block boundaries and line breaks. */
  lines: number[];
  /** Declared sizes of the images that will load; blocked ones take no real space. */
  images: { width: number; height: number }[];
  /** The page's own padding (see `baseCss`). */
  inset: { x: number; y: number };
}

const LINE_HEIGHT = 21;
const CHAR_WIDTH = 7;

/** A frame's likely size at a given width. Only a first guess: the frame measures itself. */
export function estimateSize(
  { lines, images, inset }: SizeHints,
  maxWidth: number,
): { width: number; height: number } {
  const inner = Math.max(maxWidth - inset.x * 2, CHAR_WIDTH);
  const perLine = Math.floor(inner / CHAR_WIDTH);
  let height = inset.y * 2;
  for (const chars of lines) height += Math.max(1, Math.ceil(chars / perLine)) * LINE_HEIGHT;
  for (const img of images) height += Math.round(img.height * Math.min(1, inner / img.width));
  return {
    width: Math.max(0, ...lines) * CHAR_WIDTH + inset.x * 2,
    height: Math.max(height, LINE_HEIGHT),
  };
}

const TEXT_BLOCKS =
  "address,article,aside,blockquote,center,dd,div,dl,dt,footer,h1,h2,h3,h4,h5,h6,header,li,ol,p,pre,section,table,tr,ul";

function textLines(body: HTMLElement): number[] {
  const lines: number[] = [];
  let block: Element | null = null;
  let run = 0;
  const walker = body.ownerDocument.createTreeWalker(
    body,
    NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT,
  );
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node.nodeName === "BR") {
      // After text it ends the line; on its own it is a blank one.
      lines.push(run);
      run = 0;
    } else if (node.nodeType === Node.TEXT_NODE && node.parentElement?.nodeName !== "STYLE") {
      const chars = (node.textContent ?? "").replace(/\s+/g, " ").trim().length;
      if (!chars) continue;
      const parent = node.parentElement?.closest(TEXT_BLOCKS) ?? null;
      if (parent !== block && run) {
        lines.push(run);
        run = 0;
      }
      block = parent;
      run += chars;
    }
  }
  if (run) lines.push(run);
  return lines;
}

/** Containers mail clients wrap quoted text in. Outlook: everything from the reply header on. */
const QUOTE_SELECTORS = [
  ".gmail_quote",
  ".gmail_extra",
  "blockquote[type='cite']",
  ".yahoo_quoted",
  ".moz-cite-prefix",
  "#mail-editor-reference-message-container",
  "div[data-marker='__QUOTED_TEXT__']",
];
const OUTLOOK_REPLY_START = ["#appendonsend", "#divRplyFwdMsg", "hr#stopSpelling"];

const URL_IN_CSS = /url\(\s*['"]?\s*(https?:\/\/[^'")\s]+)/gi;

/** Second-level labels under which registrations happen (co.uk, com.ua, …). */
const SECOND_LEVEL = new Set(["co", "com", "org", "net", "ac", "gov", "edu", "or", "ne", "go"]);

/** Registrable domain, e.g. "img.news.shop.co.uk" → "shop.co.uk". Good enough for rules. */
export function baseDomain(host: string): string {
  const parts = host.toLowerCase().replace(/\.$/, "").split(".");
  if (parts.length <= 2) return parts.join(".");
  const sld = parts[parts.length - 2];
  const take = SECOND_LEVEL.has(sld) && parts[parts.length - 1].length === 2 ? 3 : 2;
  return parts.slice(-take).join(".");
}

export function domainMatches(host: string, domain: string): boolean {
  const h = host.toLowerCase();
  const d = domain.toLowerCase();
  return h === d || h.endsWith(`.${d}`);
}

function hostOf(url: string): string | null {
  try {
    const u = new URL(url);
    return u.protocol === "http:" || u.protocol === "https:" ? u.hostname : null;
  } catch {
    return null;
  }
}

function remoteUrls(doc: Document): string[] {
  const urls: string[] = [];
  doc.querySelectorAll<HTMLElement>("[src], [background], [srcset], [poster]").forEach((el) => {
    for (const attr of ["src", "background", "poster"]) {
      const v = el.getAttribute(attr);
      if (v) urls.push(v);
    }
    el.getAttribute("srcset")
      ?.split(",")
      .forEach((c) => urls.push(c.trim().split(/\s+/)[0]));
  });
  const css = [
    ...Array.from(doc.querySelectorAll("style"), (s) => s.textContent ?? ""),
    ...Array.from(
      doc.querySelectorAll<HTMLElement>("[style]"),
      (e) => e.getAttribute("style") ?? "",
    ),
  ].join("\n");
  for (const m of css.matchAll(URL_IN_CSS)) urls.push(m[1]);
  return urls.filter((u) => /^https?:\/\//i.test(u.trim()));
}

function stripQuotes(doc: Document): boolean {
  let removed = false;
  for (const sel of OUTLOOK_REPLY_START) {
    const start = doc.querySelector(sel);
    if (!start) continue;
    let node: ChildNode | null = start;
    while (node) {
      const next: ChildNode | null = node.nextSibling;
      node.remove();
      node = next;
    }
    removed = true;
  }
  doc.querySelectorAll(QUOTE_SELECTORS.join(",")).forEach((el) => {
    el.remove();
    removed = true;
  });
  return removed;
}

/** Content or styling that makes an element visible even without text. */
const VISIBLE = "img,svg,hr,picture,[style],[bgcolor],[background],[height],[width]";

function isBlank(node: ChildNode): boolean {
  if (node.nodeType === Node.TEXT_NODE) return !node.textContent?.trim();
  if (node.nodeType !== Node.ELEMENT_NODE) return true;
  const el = node as Element;
  return !el.textContent?.trim() && !el.matches(VISIBLE) && !el.querySelector(VISIBLE);
}

/**
 * Removes blank lines at the edges: Apple Mail's trailing `<br>`, and the empty wrappers a
 * removed quote leaves behind. Descends into the first/last element that has content.
 */
function trimEdges(el: Element, end: "first" | "last") {
  let node = end === "first" ? el.firstChild : el.lastChild;
  while (node) {
    const next: ChildNode | null = end === "first" ? node.nextSibling : node.previousSibling;
    if (isBlank(node)) {
      node.remove();
      node = next;
      continue;
    }
    if (node.nodeType === Node.ELEMENT_NODE && !(node as Element).closest("table, pre")) {
      trimEdges(node as Element, end);
    }
    break;
  }
}

function cspFor(policy: RemotePolicy): string {
  const sources =
    policy.kind === "all"
      ? "https: http:"
      : policy.kind === "hosts"
        ? policy.domains
            .flatMap((d) => [`https://${d}`, `https://*.${d}`, `http://${d}`, `http://*.${d}`])
            .join(" ")
        : "";
  return [
    "default-src 'none'",
    `img-src data: ${sources}`,
    `style-src 'unsafe-inline' ${sources}`,
    `font-src data: ${sources}`,
    `media-src ${sources || "'none'"}`,
  ].join("; ");
}

const FONT_STACK = `-apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif`;

/**
 * Beats the email's own `!important` rules on classes and ids: `:not(#id)` adds id-level
 * specificity, so the reset wins over e.g. `.MsoNormal{font-family:Times!important}`.
 */
const STRONG = ":root:not(#_mailbub1):not(#_mailbub2)";

/**
 * The frame is sized to the document, so it must not size itself to the frame
 * (`html,body{height:100%}`), and nothing may be wider than it: fixed widths pasted from
 * web pages would be cut off. A width the sender capped on purpose keeps its own cap.
 */
const FIT_FRAME = `${STRONG},${STRONG} body{height:auto!important;min-height:0!important}
body *{max-width:100%}`;

function baseCss(display: Display, colors?: BubbleColors): string {
  if (display === "original") {
    // As the sender designed it: most HTML mail assumes a white page.
    return `html,body{margin:0;background:#fff;color:#1b1d21}
body{padding:12px;font:14px/1.45 ${FONT_STACK};overflow-wrap:anywhere}
img{max-width:100%;height:auto}
${FIT_FRAME}`;
  }
  // Readable: the app's colours and font, the sender's structure (tables, lists, images).
  const text = colors?.text || "#483d3f";
  const link = colors?.link || "#0467a0";
  // `!important` on the page itself too: the email's own `body{background:#f4f4f4}` comes
  // later in the document and would otherwise leave the app's text colour on a light page,
  // and its `body{width:100%}` plus the card's padding would be wider than the frame.
  return `${STRONG},${STRONG} body{margin:0!important;padding:0!important;box-sizing:border-box!important;background:transparent!important;color:${text}!important}
${FIT_FRAME}
${STRONG} body{font:14px/1.5 ${FONT_STACK}!important;overflow-wrap:anywhere;${display === "card" ? "padding:14px 16px!important;" : ""}}
${STRONG} body *{color:inherit!important;background-color:transparent!important;background-image:none!important;font-family:inherit!important;text-shadow:none!important;box-shadow:none!important;border-color:rgba(127,127,127,.3)!important}
${STRONG} body a,${STRONG} body a *{color:${link}!important;text-decoration:underline}
${STRONG} body img{max-width:100%!important;height:auto!important}
${STRONG} body table{max-width:100%!important}
p{margin:0 0 .6em}
blockquote{margin:.4em 0;padding-left:.6em;border-left:2px solid currentColor;opacity:.75}
${STRONG} body pre,${STRONG} body code{white-space:pre-wrap;font-family:ui-monospace,Menlo,monospace!important}${
    display === "bubble"
      ? // Plain text has no layout to keep: fixed widths pasted from web pages would overflow.
        `\n${STRONG} body *{width:auto!important;min-width:0!important;max-width:100%!important}`
      : ""
  }`;
}

const SANITIZE = {
  WHOLE_DOCUMENT: true,
  ADD_TAGS: ["style"],
  // No "target": the frame may not open popups, so a target="_blank" link would do nothing.
  ADD_ATTR: [
    "bgcolor",
    "background",
    "align",
    "valign",
    "cellpadding",
    "cellspacing",
    "border",
    "color",
    "face",
  ],
  FORBID_TAGS: [
    "script",
    "iframe",
    "frame",
    "object",
    "embed",
    "form",
    "input",
    "button",
    "textarea",
    "select",
    "base",
    "meta",
    "link",
    "video",
    "audio",
  ],
};

/** Which remote domains may load for this message, given the user's rules. */
export function decidePolicy(
  domains: string[],
  rules: { kind: string; value: string }[],
  sender: string,
  allowedOnce: boolean,
): RemotePolicy {
  if (allowedOnce) return { kind: "all" };
  const senderDomain = sender.split("@")[1] ?? "";
  const trusted = rules.some(
    (r) =>
      (r.kind === "sender" && r.value === sender.toLowerCase()) ||
      (r.kind === "domain" && senderDomain && domainMatches(senderDomain, r.value)),
  );
  if (trusted) return { kind: "all" };
  const hosts = domains.filter((d) =>
    rules.some((r) => r.kind === "host" && domainMatches(d, r.value)),
  );
  return hosts.length ? { kind: "hosts", domains: hosts } : { kind: "none" };
}

/** Remote domains still blocked under a policy. */
export function blockedDomains(domains: string[], policy: RemotePolicy): string[] {
  if (policy.kind === "all") return [];
  if (policy.kind === "none") return domains;
  return domains.filter((d) => !policy.domains.includes(d));
}

export function prepareHtml(
  raw: string,
  opts: {
    /** Chosen after scanning, from the remote domains the message uses. */
    policy: (remoteDomains: string[]) => RemotePolicy;
    stripQuotes: boolean;
    /** Readable mode: the app's colours and font instead of the sender's. */
    readable: boolean;
    colors?: BubbleColors;
    /** The app's theme, which the frame is set to as well. */
    scheme: "light" | "dark";
  },
): PreparedHtml & { policy: RemotePolicy } {
  const clean = DOMPurify.sanitize(raw, SANITIZE) as unknown as string;
  const doc = new DOMParser().parseFromString(clean, "text/html");

  const probe = new DOMParser().parseFromString(clean, "text/html");
  const hasQuote = stripQuotes(probe);
  if (opts.stripQuotes) stripQuotes(doc);
  trimEdges(doc.body, "first");
  trimEdges(doc.body, "last");

  const domains = new Set<string>();
  for (const url of remoteUrls(doc)) {
    const host = hostOf(url.trim());
    if (host) domains.add(baseDomain(host));
  }

  const rich = Boolean(
    doc.querySelector("table, img, [bgcolor], [background]") ||
    /background(-color|-image)?\s*:/i.test(
      Array.from(doc.querySelectorAll("style"), (s) => s.textContent).join(""),
    ),
  );

  doc.querySelectorAll<HTMLAnchorElement>("a[href]").forEach((a) => {
    // Hovering shows where a link really goes.
    a.title = a.getAttribute("href") ?? "";
  });

  // Before the sizes below are moved out of the attributes.
  const declaredImages = Array.from(doc.images, (img) => ({
    src: img.getAttribute("src") ?? "",
    width: parseFloat(img.getAttribute("width") ?? ""),
    height: parseFloat(img.getAttribute("height") ?? ""),
  })).filter((img) => img.width > 0 && img.height > 0);

  doc.querySelectorAll<HTMLImageElement>("img[width]").forEach((img) => {
    // A table cell never gets narrower than an image's fixed width, even one capped with
    // max-width:100%, so a 660px header pushed the whole layout (and its text) out of the
    // frame. A percentage width is allowed to shrink; the attribute stays the upper bound.
    const width = parseFloat(img.getAttribute("width") ?? "");
    if (!(width > 0)) return;
    img.style.setProperty("width", "100%", "important");
    img.style.setProperty("max-width", `${width}px`, "important");
    img.style.setProperty("height", "auto", "important");
    img.removeAttribute("width");
    img.removeAttribute("height");
  });

  const remoteDomains = [...domains].sort();
  const policy = opts.policy(remoteDomains);
  const head = doc.head;
  const csp = doc.createElement("meta");
  csp.httpEquiv = "Content-Security-Policy";
  csp.content = cspFor(policy);
  const display: Display = !opts.readable ? "original" : rich ? "card" : "bubble";
  const blocked = blockedDomains(remoteDomains, policy);
  const sizeHints: SizeHints = {
    lines: textLines(doc.body),
    images: declaredImages
      .filter(({ src }) => {
        const host = hostOf(src.trim());
        return !host || !blocked.includes(baseDomain(host));
      })
      .map(({ width, height }) => ({ width, height })),
    inset:
      display === "card"
        ? { x: 16, y: 14 }
        : display === "original"
          ? { x: 12, y: 12 }
          : { x: 0, y: 0 },
  };
  const scheme = doc.createElement("meta");
  scheme.name = "color-scheme";
  // Original: the sender's design on white. Readable: the frame's scheme, spelled out, as a
  // document whose scheme differs from its frame's gets an opaque page behind it, and
  // "light dark" would follow the system, which can differ from the app's theme.
  scheme.content = display === "original" ? "light" : opts.scheme;
  const style = doc.createElement("style");
  style.textContent = baseCss(display, opts.colors);
  // The CSP must come first so it applies before any image in the head is parsed.
  head.prepend(csp, scheme, style);

  return {
    srcdoc: `<!doctype html>${doc.documentElement.outerHTML}`,
    display,
    rich,
    remoteDomains,
    hasQuote,
    sizeHints,
    policy,
  };
}

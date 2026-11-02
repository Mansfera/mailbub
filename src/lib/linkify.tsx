import type { ReactNode } from "react";
import { openLink } from "./openLink";

const LINK = /\b(?:https?:\/\/[^\s<>"]+|www\.[^\s<>"]+|[\w.+-]+@[\w-]+(?:\.[\w-]+)+)/gi;
/** Punctuation that usually ends a sentence rather than the URL. */
const TRAILING = /[.,;:!?)\]'"»]+$/;

/** Plain text with URLs and addresses turned into links that open in the browser/mail app. */
export function linkify(text: string, className: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  for (const m of text.matchAll(LINK)) {
    const start = m.index ?? 0;
    let raw = m[0];
    const trail = raw.match(TRAILING)?.[0] ?? "";
    raw = raw.slice(0, raw.length - trail.length);
    if (start > last) out.push(text.slice(last, start));
    const href =
      raw.includes("@") && !raw.includes("/")
        ? `mailto:${raw}`
        : raw.startsWith("www.")
          ? `https://${raw}`
          : raw;
    out.push(
      <a
        key={start}
        href={href}
        title={href}
        className={className}
        onClick={(e) => {
          e.preventDefault();
          e.stopPropagation();
          openLink(href);
        }}
      >
        {raw}
      </a>,
    );
    last = start + raw.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

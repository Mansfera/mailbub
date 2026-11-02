import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { core, IN_TAURI } from "../lib/core";
import { estimateSize, type SizeHints } from "../lib/emailHtml";
import { openLink } from "../lib/openLink";

/** The context menu of the email frame under the pointer, if any. */
let hovered: (() => void) | null = null;
// The core empties WebKit's own menu in an email frame and asks for ours instead.
if (IN_TAURI) void listen("mail-frame-menu", () => hovered?.());

/** Sizes frames last measured themselves at, so a chat opened again starts out right. */
const measured = new Map<string, { width: number; height: number }>();

/** The width a frame gets: the caps of the bubble and card in `Conversation`. */
function availableWidth(fit: boolean | undefined, wide: boolean | undefined): number {
  const column = Math.min(window.innerWidth - (wide ? 16 : 40), 760);
  if (wide) return column;
  return fit ? Math.min(560, column * 0.78) - 24 : Math.min(680, column * 0.92);
}

/** Width the text takes on one line per paragraph; the parent caps it via max-width. */
function textWidth(doc: Document): number {
  doc.body.style.width = "max-content";
  const width = Math.ceil(doc.body.getBoundingClientRect().width);
  doc.body.style.width = "";
  return width;
}

/**
 * Shows a prepared email document in an iframe sandboxed without scripts.
 * `allow-same-origin` (never combined with `allow-scripts`) lets this component size the
 * frame. WebKit never runs listeners inside a script-less frame, so in the app a clicked
 * link navigates the frame and the core's navigation hook opens it in the browser, and a
 * right-click is caught by the core while the pointer is over the frame. The click
 * listener below covers Chromium (the browser dev build).
 */
export function HtmlBody({
  srcdoc,
  sizeKey,
  sizeHints,
  colorScheme,
  fit,
  wide,
  onContextMenu,
}: {
  srcdoc: string;
  /** Identifies this layout of the message, across visits to the chat. */
  sizeKey: string;
  /** Reserves the frame's space on first sight, until the document has laid itself out. */
  sizeHints: SizeHints;
  /** Shrink to the text's own width (up to the parent's), so short mail gets a short bubble. */
  fit?: boolean;
  /** Takes the whole column rather than a card's width. */
  wide?: boolean;
  /** Right-clicks inside the frame never reach the parent's handlers; this gets them. */
  onContextMenu?: (selection: string) => void;
  /** "light" stops the email's own dark-mode CSS from turning text pale on a white page. */
  colorScheme: "light" | "dark";
}) {
  const ref = useRef<HTMLIFrameElement>(null);
  const observer = useRef<ResizeObserver | null>(null);
  const menu = useRef(onContextMenu);
  useEffect(() => {
    menu.current = onContextMenu;
  }, [onContextMenu]);
  const [start] = useState(
    () => measured.get(sizeKey) ?? estimateSize(sizeHints, availableWidth(fit, wide)),
  );
  const [height, setHeight] = useState(start.height);
  const [width, setWidth] = useState(start.width);
  const sized = useRef<Document | null>(null);
  const remember = (size: { width: number } | { height: number }) =>
    measured.set(sizeKey, { ...start, ...measured.get(sizeKey), ...size });

  const showMenu = useRef(() =>
    menu.current?.(ref.current?.contentDocument?.getSelection()?.toString().trim() ?? ""),
  );
  const setHovered = (over: boolean) => {
    if (!IN_TAURI || (over && !menu.current)) return;
    if (!over && hovered !== showMenu.current) return;
    hovered = over ? showMenu.current : null;
    void core.setMailFrameHover(over).catch(() => undefined);
  };

  useEffect(
    () => () => {
      observer.current?.disconnect();
      if (hovered === showMenu.current) {
        hovered = null;
        void core.setMailFrameHover(false).catch(() => undefined);
      }
    },
    [],
  );

  const sizeToContent = (doc: Document) => {
    if (sized.current === doc) return;
    sized.current = doc;
    // Not scrollHeight: it never drops below the frame's own height, so a frame that was
    // once tall (laid out narrow before `fit` widened it) would keep the empty space.
    const measure = () => {
      const measuredHeight = Math.ceil(doc.documentElement.getBoundingClientRect().height);
      remember({ height: measuredHeight });
      setHeight(measuredHeight);
    };
    measure();
    observer.current?.disconnect();
    observer.current = new ResizeObserver(measure);
    observer.current.observe(doc.documentElement);
  };

  // `load` waits for remote images; the text's height is known as soon as it is parsed.
  const parsing = useRef(0);
  useEffect(() => {
    const poll = () => {
      const doc = ref.current?.contentDocument;
      if (doc?.body && doc.URL === "about:srcdoc") sizeToContent(doc);
      else parsing.current = requestAnimationFrame(poll);
    };
    parsing.current = requestAnimationFrame(poll);
    return () => cancelAnimationFrame(parsing.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [srcdoc]);

  const onLoad = () => {
    cancelAnimationFrame(parsing.current);
    const doc = ref.current?.contentDocument;
    if (!doc) return;
    if (fit) {
      const measuredWidth = textWidth(doc);
      remember({ width: measuredWidth });
      setWidth(measuredWidth);
    }
    sizeToContent(doc);
    doc.addEventListener("click", (e) => {
      const a = (e.target as Element | null)?.closest?.("a[href]");
      if (!a) return;
      e.preventDefault();
      openLink(a.getAttribute("href") ?? "");
    });
  };

  return (
    <iframe
      ref={ref}
      title="Message"
      sandbox="allow-same-origin"
      srcDoc={srcdoc}
      onLoad={onLoad}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{ height, width: fit ? width : undefined, colorScheme }}
      className={`block border-0 ${fit ? "max-w-full" : "w-full"}`}
    />
  );
}

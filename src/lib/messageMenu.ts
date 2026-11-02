import type { TFunction } from "i18next";
import type { Message } from "../types";
import type { MoveItem } from "./mailbox";
import { popupMenu } from "./popupMenu";

type Toggle<K extends string> = { [key in K]: boolean } & { toggle: () => void };

/** Context menu for one message bubble. */
export async function openMessageMenu(
  m: Message,
  t: TFunction,
  opts: {
    /** Starts a reply to this message; null where a reply isn't possible. */
    reply: (() => void) | null;
    /** Quoted text exists and can be shown or hidden. */
    quote: Toggle<"expanded"> | null;
    /** HTML message: switch between the sender's design and the readable version. */
    view: Toggle<"original"> | null;
    /** Text selected in the email frame, which WebKit's own Copy item would have copied. */
    selection?: string;
    /** "Move to" submenu: the account's folders. */
    moveTo?: MoveItem[] | null;
  },
) {
  const { reply, quote, view, selection, moveTo } = opts;
  await popupMenu([
    ...(reply ? [{ text: t("message.reply"), action: reply }] : []),
    ...(selection
      ? [{ text: t("message.copy"), action: () => void navigator.clipboard.writeText(selection) }]
      : []),
    ...(quote
      ? [
          {
            text: t(quote.expanded ? "conversation.hideFull" : "conversation.showFull"),
            action: quote.toggle,
          },
        ]
      : []),
    ...(view
      ? [
          {
            text: t(view.original ? "message.showReadable" : "message.showOriginal"),
            action: view.toggle,
          },
        ]
      : []),
    ...(reply || quote || view ? [{ item: "Separator" as const }] : []),
    {
      text: t("message.copyText"),
      action: () => void navigator.clipboard.writeText(m.body),
    },
    ...(moveTo?.length
      ? [{ item: "Separator" as const }, { text: t("message.moveTo"), items: moveTo }]
      : []),
  ]);
}

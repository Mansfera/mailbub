import type { TFunction } from "i18next";
import { useMail } from "../store";
import { effectiveStatus } from "./chat";
import { core } from "./core";
import { moveMenuItems } from "./mailbox";
import { popupMenu } from "./popupMenu";
import type { Chat, ChatStatus } from "../types";

const STATUSES: ChatStatus[] = ["needs_reply", "waiting", "done"];

/** Context menu for a chat row. */
export async function openChatMenu(chat: Chat, t: TFunction) {
  const m = useMail.getState();
  const current = effectiveStatus(chat);
  // The whole conversation; its messages are fetched only if it was never opened.
  const moveTo = await moveMenuItems(chat.accountId, async () =>
    (m.messages[chat.id] ?? (await core.chatMessages(chat.id))).filter((msg) => !msg.delivery),
  );
  await popupMenu([
    {
      text: t(chat.unread ? "conversation.markRead" : "conversation.markUnread"),
      action: () => (chat.unread ? m.markRead(chat.id) : m.markUnread(chat.id)),
    },
    {
      text: t(chat.pinned ? "conversation.unpin" : "conversation.pin"),
      action: () => m.togglePin(chat.id),
    },
    {
      text: t(chat.muted ? "conversation.unmute" : "conversation.mute"),
      action: () => m.toggleMute(chat.id),
    },
    { item: "Separator" },
    {
      text: t("conversation.status"),
      items: STATUSES.map((st) => ({
        text: t(`status.${st}`),
        checked: current === st,
        action: () => m.setStatus(chat.id, st),
      })),
    },
    { item: "Separator" },
    {
      text: t(chat.archived ? "conversation.unarchive" : "conversation.archive"),
      action: () => m.toggleArchive(chat.id),
    },
    { text: t("message.moveTo"), items: moveTo },
    {
      text: t("conversation.delete"),
      action: () => m.deleteChat(chat.id),
    },
  ]);
}

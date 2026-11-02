import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, MailMinus, X } from "lucide-react";
import { useMail, usePrefs } from "../store";
import { chatDisplayName } from "../lib/chat";
import { core } from "../lib/core";
import { openLink } from "../lib/openLink";
import { unsubscribeAction } from "../lib/unsubscribe";
import { IconButton } from "./primitives";
import type { Chat } from "../types";

/**
 * A strip under the header of a chat with a mailing list: offers to unsubscribe, and can be
 * dismissed for good. Unsubscribing archives the chat (Undo brings it back). One-click lists
 * are left right here; others open their page or a prepared email.
 */
export function UnsubscribeBanner({ chat }: { chat: Chat }) {
  const { t } = useTranslation();
  const messages = useMail((s) => s.messages[chat.id]);
  const hidden = usePrefs((p) => p.unsubscribeHidden.includes(chat.id));
  const hide = usePrefs((p) => p.hideUnsubscribe);
  const [working, setWorking] = useState(false);
  const name = chatDisplayName(chat);

  const action = useMemo(() => {
    if (!messages) return null;
    // Footer links only where nobody expects a conversation: machine mail, or a chat you
    // never wrote in (cached mail from before the automated flag has it unset).
    const links = chat.automated || !messages.some((m) => m.outgoing);
    for (let i = messages.length - 1; i >= 0; i--) {
      const a = unsubscribeAction(messages[i], { links });
      if (a) return a;
    }
    return null;
  }, [messages, chat.automated]);

  if (hidden || !action) return null;

  /** Done with this list: the strip goes for good and the chat leaves the inbox. */
  const finish = (toast: string) => {
    hide(chat.id);
    const { toggleArchive, notify } = useMail.getState();
    if (chat.archived) notify(t(toast));
    else toggleArchive(chat.id, toast);
  };

  const unsubscribe = async () => {
    if (action.kind !== "oneClick") {
      openLink(action.url);
      finish(action.kind === "web" ? "unsubscribe.finishInBrowser" : "unsubscribe.finishInMail");
      return;
    }
    setWorking(true);
    try {
      await core.unsubscribeOneClick(action.url);
      finish("unsubscribe.done");
    } catch (e) {
      setWorking(false);
      useMail.getState().notify(`${t("unsubscribe.failed")}: ${(e as Error).message}`);
    }
  };

  return (
    <div className="flex shrink-0 items-center gap-2.5 border-b border-line px-4 py-2 text-[12px]">
      <MailMinus size={16} aria-hidden className="shrink-0 text-text-2" />
      <div className="min-w-0 flex-1 truncate">
        <span className="font-medium">{t("unsubscribe.title")}</span>{" "}
        <span className="text-text-2">{t("unsubscribe.hint", { name })}</span>
      </div>
      <button
        type="button"
        disabled={working}
        onClick={() => void unsubscribe()}
        className="flex h-7 shrink-0 items-center gap-1.5 rounded-md bg-accent px-3 font-medium text-on-accent hover:bg-accent-hover disabled:opacity-70"
      >
        {working && <Loader2 size={13} aria-hidden className="animate-spin" />}
        {t(working ? "unsubscribe.working" : "unsubscribe.action")}
      </button>
      <IconButton label={t("unsubscribe.dismiss")} onClick={() => hide(chat.id)}>
        <X size={15} />
      </IconButton>
    </div>
  );
}

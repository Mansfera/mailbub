import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Search } from "lucide-react";
import { useMail, usePrefs } from "../store";
import { chatDisplayName, isUnsubscribeChat, lastMessage, normalizeSubject } from "../lib/chat";
import { matchesSearch, searchOrder, snippetFor, useChatSearch } from "../lib/search";
import { AccountDot, Avatar, Snippet } from "./primitives";
import { Modal } from "./Modal";
import { useMailbox } from "../lib/mailbox";

export function QuickSwitcher() {
  const { t } = useTranslation();
  const chats = useMail((s) => s.chats);
  const showUnsubscribe = usePrefs((p) => p.showUnsubscribe);
  const accounts = useMail((s) => s.accounts);
  const { select, setSwitcherOpen } = useMail.getState();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const search = useChatSearch(query);

  const results = useMemo(() => {
    const byRank = searchOrder(query, search);
    return chats
      .filter((c) => showUnsubscribe || !isUnsubscribeChat(c))
      .filter((c) => matchesSearch(c, query, search))
      .sort(
        byRank ??
          ((a, b) => (lastMessage(b)?.sentAt ?? "").localeCompare(lastMessage(a)?.sentAt ?? "")),
      )
      .slice(0, 8);
  }, [chats, query, search, showUnsubscribe]);
  const close = () => setSwitcherOpen(false);
  const choose = (i: number) => {
    const chat = results[i];
    if (!chat) return;
    // Back from a browsed mailbox (Trash, …) to where the chat lives in the cache.
    void useMailbox.getState().open({ kind: chat.archived ? "archive" : "inbox" });
    select(chat.id);
    close();
  };

  return (
    <Modal onClose={close} label={t("switcher.placeholder")} className="top-[14vh] w-[520px]">
      <div className="flex h-11 items-center gap-2 border-b border-line px-3">
        <Search size={16} className="text-text-3" aria-hidden />
        <input
          autoFocus
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) {
              e.preventDefault();
              setActive((a) => Math.min(a + 1, results.length - 1));
            } else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) {
              e.preventDefault();
              setActive((a) => Math.max(a - 1, 0));
            } else if (e.key === "Enter") {
              e.preventDefault();
              choose(active);
            }
          }}
          placeholder={t("switcher.placeholder")}
          className="flex-1 bg-transparent text-[14px] outline-none placeholder:text-text-3"
        />
      </div>
      <ul role="listbox" className="max-h-[360px] overflow-y-auto p-1.5">
        {results.length === 0 && (
          <li className="px-3 py-6 text-center text-text-2">{t("switcher.noResults")}</li>
        )}
        {results.map((chat, i) => {
          const account = accounts.find((a) => a.id === chat.accountId);
          const last = lastMessage(chat);
          const snippet = snippetFor(chat, query, search);
          return (
            <li
              key={chat.id}
              role="option"
              aria-selected={i === active}
              onMouseMove={() => setActive(i)}
              onClick={() => choose(i)}
              className={`flex items-center gap-2.5 rounded-lg px-2 py-1.5 ${i === active ? "bg-selected" : ""}`}
            >
              <Avatar people={chat.participants} size={28} />
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px] font-medium">{chatDisplayName(chat)}</div>
                <div className="truncate text-[12px] text-text-2">
                  {snippet ? <Snippet text={snippet} /> : last && normalizeSubject(last.subject)}
                </div>
              </div>
              {account && (
                <span className="flex shrink-0 items-center gap-1.5 text-[11px] text-text-2">
                  <AccountDot color={account.color} />
                  {account.label}
                </span>
              )}
            </li>
          );
        })}
      </ul>
    </Modal>
  );
}

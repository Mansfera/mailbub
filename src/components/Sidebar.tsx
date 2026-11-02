import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  Archive,
  BellOff,
  ChevronDown,
  Clock,
  CornerUpLeft,
  FilePen,
  Folder,
  Inbox,
  Mail,
  MailOpen,
  Pin,
  Plus,
  RotateCw,
  Search,
  Send,
  ShieldAlert,
  Settings,
  SquarePen,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import { popupMenu } from "../lib/popupMenu";
import type { TFunction } from "i18next";
import { useMail, usePrefs, visibleChats, type StatusFilter } from "../store";
import {
  chatDisplayName,
  effectiveStatus,
  lastMessage,
  normalizeSubject,
  unreadCount,
} from "../lib/chat";
import { formatListDate, formatTime } from "../lib/format";
import { snippetFor } from "../lib/search";
import { openChatMenu } from "../lib/chatMenu";
import { IN_TAURI } from "../lib/core";
import { SWIPE_THRESHOLD, useSwipe } from "../lib/useSwipe";
import { AccountDot, Avatar, IconButton, Kbd, Snippet } from "./primitives";
import { openSettingsWindow } from "../lib/settingsWindow";
import {
  BROWSE_ROLES,
  browsedChats,
  isBrowsedChat,
  mailboxLabel,
  moveMenuItems,
  useMailbox,
  type View,
} from "../lib/mailbox";
import type { Account, Chat, Mailbox } from "../types";

const STATUS_FILTERS: StatusFilter[] = ["all", "needs_reply", "waiting", "done"];

export function Sidebar() {
  const { t } = useTranslation();
  const s = useMail(
    useShallow((s) => ({
      chats: s.chats,
      accounts: s.accounts,
      accountFilter: s.accountFilter,
      statusFilter: s.statusFilter,
      showArchived: s.showArchived,
      query: s.query,
      search: s.search,
      selectedId: s.selectedId,
      accountsLoaded: s.accountsLoaded,
    })),
  );
  const { select, setAccountFilter, setStatusFilter, setQuery, setComposeOpen, setSetup } =
    useMail.getState();
  const hasAccounts = s.accounts.length > 0;

  const showUnsubscribe = usePrefs((p) => p.showUnsubscribe);
  const view = useMailbox((m) => m.view);
  const browsedAll = useMailbox((m) => m.chats);
  const browsing = useMailbox((m) => m.loading);
  // Trash, Junk, … are read live from the server; Inbox and Archive come from the cache.
  const remote = view.kind === "role" || view.kind === "folder";
  // Inbox vs Archive follows the cache's own flag, which the quick switcher also sets.
  const shown: View = remote ? view : { kind: s.showArchived ? "archive" : "inbox" };
  const list = useMemo(
    () => (remote ? browsedChats(browsedAll, s) : visibleChats(s, showUnsubscribe)),
    [remote, browsedAll, s, showUnsubscribe],
  );
  const counts = useMemo(() => {
    const base = visibleChats({ ...s, statusFilter: "all" }, showUnsubscribe);
    const out: Record<StatusFilter, number> = {
      all: base.length,
      needs_reply: 0,
      waiting: 0,
      done: 0,
    };
    for (const c of base) out[effectiveStatus(c)]++;
    return out;
  }, [s, showUnsubscribe]);
  const accountById = useMemo(() => new Map(s.accounts.map((a) => [a.id, a])), [s.accounts]);

  return (
    <aside
      className={`flex w-[320px] shrink-0 flex-col border-r border-sidebar-line bg-sidebar ${
        s.selectedId || (s.accountsLoaded && !hasAccounts)
          ? "max-[700px]:hidden"
          : "max-[700px]:w-full max-[700px]:border-r-0"
      }`}
    >
      {/* Title-bar strip: room for the traffic lights, draggable. */}
      <div
        data-tauri-drag-region
        className="flex h-[52px] shrink-0 items-center justify-end gap-1 pr-3 pl-[84px] max-[700px]:pl-3"
      >
        {hasAccounts && <MailboxPicker accounts={s.accounts} view={shown} />}
        {hasAccounts && <SyncButton accounts={s.accounts} />}
        <IconButton
          label={`${t("sidebar.compose")} (⌘N)`}
          onClick={() => (hasAccounts ? setComposeOpen(true) : setSetup("choose"))}
        >
          <SquarePen size={16} />
        </IconButton>
      </div>

      <div className="px-3">
        <label className="flex h-7 items-center gap-1.5 rounded-md bg-hover px-2 text-text-3 focus-within:ring-2 focus-within:ring-brand">
          <Search size={14} aria-hidden />
          <input
            id="sidebar-search"
            value={s.query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setQuery("");
                e.currentTarget.blur();
              }
            }}
            placeholder={t("sidebar.search")}
            className="min-w-0 flex-1 bg-transparent text-[13px] text-text outline-none placeholder:text-text-3"
          />
          <Kbd>⌘K</Kbd>
        </label>
      </div>

      <AccountFilter
        accounts={s.accounts}
        value={s.accountFilter}
        onChange={setAccountFilter}
        onAdd={() => setSetup("choose")}
      />

      <div
        hidden={!hasAccounts || remote}
        role="tablist"
        aria-label={t("sidebar.statusFilter")}
        className="flex border-b border-sidebar-line px-3 pb-2"
      >
        <div className="flex w-full gap-0.5 overflow-x-auto rounded-lg bg-hover p-0.5 [scrollbar-width:none]">
          {STATUS_FILTERS.map((f) => (
            <button
              key={f}
              role="tab"
              aria-selected={s.statusFilter === f}
              onClick={() => setStatusFilter(f)}
              className={`flex h-6 shrink-0 grow items-center justify-center gap-1 rounded-md px-2 text-[12px] whitespace-nowrap ${
                s.statusFilter === f
                  ? "bg-raised font-medium text-text shadow-sm"
                  : "text-text-2 hover:text-text"
              }`}
            >
              {t(`status.${f}`)}
              {/* Counts only where they mean work: who owes whom a reply. */}
              {(f === "needs_reply" || f === "waiting") && counts[f] > 0 && (
                <span className="text-[11px] tabular-nums text-text-3">{counts[f]}</span>
              )}
            </button>
          ))}
        </div>
      </div>

      <ul
        className="flex-1 overflow-y-auto px-2 py-1"
        role="listbox"
        aria-label={viewLabel(shown, t)}
      >
        {list.length === 0 && hasAccounts && !(remote && browsing) && (
          <li className="px-3 py-8 text-center text-text-2">{t("sidebar.empty")}</li>
        )}
        {list.map((chat) => (
          <ChatRow
            key={chat.id}
            chat={chat}
            account={accountById.get(chat.accountId)}
            selected={chat.id === s.selectedId}
            onSelect={() =>
              remote ? useMail.setState({ selectedId: chat.id, unreadFrom: null }) : select(chat.id)
            }
            snippet={remote ? undefined : snippetFor(chat, s.query, s.search)}
          />
        ))}
        {hasAccounts &&
          (remote ? <BrowseMoreRow /> : <OlderMailRow searching={Boolean(s.query.trim())} />)}
      </ul>

      <div className="flex h-10 shrink-0 items-center border-t border-sidebar-line px-2">
        <IconButton label={`${t("settings.title")} (⌘,)`} onClick={() => void openSettingsWindow()}>
          <Settings size={16} />
        </IconButton>
      </div>
    </aside>
  );
}

function AccountFilter({
  accounts,
  value,
  onChange,
  onAdd,
}: {
  accounts: Account[];
  value: string;
  onChange: (id: string) => void;
  onAdd: () => void;
}) {
  const { t } = useTranslation();
  const chip = (active: boolean) =>
    `flex h-6 items-center gap-1.5 rounded-full border px-2 text-[12px] whitespace-nowrap ${
      active ? "border-transparent bg-selected text-text" : "border-line text-text-2 hover:bg-hover"
    }`;
  return (
    <div
      role="radiogroup"
      aria-label={t("sidebar.accountFilter")}
      className="flex flex-wrap gap-1 px-3 pt-2.5 pb-2"
    >
      {accounts.length > 0 && (
        <button
          role="radio"
          aria-checked={value === "all"}
          className={chip(value === "all")}
          onClick={() => onChange("all")}
        >
          {t("sidebar.allAccounts")}
        </button>
      )}
      {accounts.map((a) => (
        <button
          key={a.id}
          role="radio"
          aria-checked={value === a.id}
          title={a.email}
          className={chip(value === a.id)}
          onClick={() => onChange(a.id)}
        >
          <AccountDot color={a.color} />
          {a.label}
        </button>
      ))}
      <button
        type="button"
        aria-label={t("sidebar.addAccount")}
        title={t("sidebar.addAccount")}
        onClick={onAdd}
        className={`${chip(false)} ${accounts.length ? "px-1.5" : ""}`}
      >
        <Plus size={13} aria-hidden />
        {accounts.length === 0 && t("sidebar.addAccount")}
      </button>
    </div>
  );
}

function ChatRow({
  chat,
  account,
  selected,
  onSelect,
  snippet,
}: {
  chat: Chat;
  account?: Account;
  selected: boolean;
  onSelect: () => void;
  /** Search excerpt shown instead of the newest message. */
  snippet?: string;
}) {
  const { t } = useTranslation();
  const last = lastMessage(chat);
  const unread = unreadCount(chat);
  const status = effectiveStatus(chat);
  const preview =
    last?.body.replace(/\s+/g, " ").trim() ||
    (last?.attachments.length ? t("chat.attachment") : "");
  const { toggleArchive, markRead, markUnread } = useMail.getState();
  // Read live from the server: only "Move to" applies, so no swipes or chat menu.
  const browsed = isBrowsedChat(chat.id);
  // Like Mail: swipe left to archive, right to toggle read.
  const {
    ref: swipeRef,
    offset,
    dragging,
  } = useSwipe<HTMLLIElement>(
    () => toggleArchive(chat.id),
    () => (unread ? markRead(chat.id) : markUnread(chat.id)),
  );
  const armed = Math.abs(offset) >= SWIPE_THRESHOLD;

  return (
    <li
      ref={browsed ? undefined : swipeRef}
      role="option"
      aria-selected={selected}
      data-chat-id={chat.id}
      onClick={onSelect}
      onContextMenu={(e) => {
        if (!IN_TAURI) return;
        e.preventDefault();
        if (!browsed) return void openChatMenu(chat, t);
        void moveMenuItems(chat.accountId, useMailbox.getState().messages[chat.id] ?? [])
          .then((items) => popupMenu([{ text: t("message.moveTo"), items }]))
          .catch((err: Error) => useMail.getState().notify(err.message));
      }}
      className="relative overflow-hidden rounded-lg"
    >
      {offset !== 0 && (
        <div
          aria-hidden
          className={`absolute inset-0 flex items-center gap-1.5 px-4 text-[12px] font-medium text-white transition-opacity ${
            offset < 0 ? "justify-end bg-account-slate" : "justify-start bg-accent"
          } ${armed ? "opacity-100" : "opacity-60"}`}
        >
          {offset < 0 ? (
            <>
              {chat.archived ? <Inbox size={15} /> : <Archive size={15} />}
              {t(chat.archived ? "conversation.unarchive" : "conversation.archive")}
            </>
          ) : (
            <>
              {unread ? <MailOpen size={15} /> : <Mail size={15} />}
              {t(unread ? "conversation.markRead" : "conversation.markUnread")}
            </>
          )}
        </div>
      )}
      <div
        style={{
          transform: offset ? `translateX(${offset}px)` : undefined,
          transition: dragging ? "none" : "transform 180ms ease-out",
        }}
        className={`relative flex gap-2.5 rounded-lg px-2.5 py-2 ${
          offset !== 0 ? "bg-raised shadow-sm" : selected ? "bg-selected" : "hover:bg-hover"
        }`}
      >
        <Avatar people={chat.participants} size={40} accountColor={account?.color} />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-1.5">
            <span className={`truncate text-[13px] ${unread ? "font-semibold" : "font-medium"}`}>
              {chatDisplayName(chat)}
            </span>
            {chat.pinned && (
              <Pin size={11} className="shrink-0 text-text-3" aria-label={t("chat.pinned")} />
            )}
            {chat.muted && (
              <BellOff size={11} className="shrink-0 text-text-3" aria-label={t("chat.muted")} />
            )}
            <span className="ml-auto shrink-0 text-[11px] tabular-nums text-text-2">
              {last && formatListDate(last.sentAt)}
            </span>
          </div>
          <div className="truncate text-[12px] text-text">
            {last && normalizeSubject(last.subject)}
          </div>
          <div className="flex items-center gap-1.5">
            <span
              className={`line-clamp-1 flex-1 text-[12px] ${unread ? "text-text" : "text-text-2"}`}
            >
              {snippet ? (
                <Snippet text={snippet} />
              ) : (
                <>
                  {last?.outgoing && <span className="text-text-3">{t("chat.you")}: </span>}
                  {preview}
                </>
              )}
            </span>
            {unread > 0 ? (
              <span
                aria-label={t("chat.unread", { count: unread })}
                className={`min-w-[18px] rounded-full px-1.5 text-center text-[11px] leading-[18px] font-semibold tabular-nums ${
                  chat.muted ? "bg-text-3 text-bg" : "bg-accent text-on-accent"
                }`}
              >
                {unread}
              </span>
            ) : status === "needs_reply" ? (
              <CornerUpLeft
                size={13}
                className="shrink-0 text-accent-text"
                aria-label={t("status.needs_reply")}
              />
            ) : status === "waiting" ? (
              <Clock size={13} className="shrink-0 text-text-3" aria-label={t("status.waiting")} />
            ) : null}
          </div>
        </div>
      </div>
    </li>
  );
}

/**
 * End of the list: fetches older mail from the server when scrolled into view (or on click).
 * While searching it waits for a click, so typing doesn't start downloads.
 */
function OlderMailRow({ searching }: { searching: boolean }) {
  const { t } = useTranslation();
  const ref = useRef<HTMLLIElement>(null);
  const loading = useMail((s) => s.olderLoading);
  const done = useMail(
    (s) =>
      s.accounts.length > 0 &&
      s.accounts
        .filter((a) => s.accountFilter === "all" || a.id === s.accountFilter)
        .every((a) => s.olderDone[a.id]),
  );
  const loadOlder = useMail((s) => s.loadOlder);

  useEffect(() => {
    const el = ref.current;
    if (!el || searching || done) return;
    const observer = new IntersectionObserver(
      ([entry]) => entry?.isIntersecting && void loadOlder(),
      { root: el.parentElement, rootMargin: "200px" },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [searching, done, loadOlder]);

  return (
    <li ref={ref} className="flex justify-center px-3 py-3 text-[12px]">
      {loading ? (
        <span className="flex items-center gap-1.5 text-text-2" role="status">
          <RotateCw size={12} className="animate-spin" aria-hidden />
          {t("older.loading")}
        </span>
      ) : done ? (
        <span className="text-text-3">{t("older.none")}</span>
      ) : (
        <button
          type="button"
          onClick={() => void loadOlder()}
          className="rounded-md px-2 py-1 text-accent-text hover:bg-hover"
        >
          {t("older.load")}
        </button>
      )}
    </li>
  );
}

function SyncButton({ accounts }: { accounts: Account[] }) {
  const { t } = useTranslation();
  const sync = useMail((s) => s.sync);
  const syncNow = useMail((s) => s.syncNow);
  const failed = sync.errors.map(
    (e) => `${accounts.find((a) => a.id === e.accountId)?.label ?? e.accountId}: ${e.message}`,
  );
  const status = sync.running
    ? t("sidebar.syncing")
    : sync.lastRun
      ? t("sidebar.lastSynced", { time: formatTime(new Date(sync.lastRun * 1000).toISOString()) })
      : null;
  const label = [
    t("sidebar.refresh"),
    status,
    ...(failed.length ? [t("sidebar.syncErrors"), ...failed] : []),
  ]
    .filter(Boolean)
    .join("\n");
  return (
    <IconButton label={label} onClick={syncNow} disabled={sync.running} className="relative">
      <RotateCw size={15} className={sync.running ? "animate-spin" : ""} />
      {failed.length > 0 && !sync.running && (
        <TriangleAlert
          size={10}
          aria-hidden
          className="absolute right-0.5 bottom-0.5 fill-bg text-account-gold"
        />
      )}
    </IconButton>
  );
}

function viewLabel(view: View, t: TFunction): string {
  switch (view.kind) {
    case "inbox":
    case "archive":
      return t(`mailbox.${view.kind}`);
    case "role":
      return t(`mailbox.${view.role}`);
    case "folder":
      return view.label;
  }
}

/**
 * Title-bar menu for what the list shows: Inbox and Archive (the synced cache), then
 * Sent, Drafts, Junk and Trash across accounts and each account's own folders, read live.
 */
function MailboxPicker({ accounts, view }: { accounts: Account[]; view: View }) {
  const { t } = useTranslation();
  const loadBoxes = useMailbox((m) => m.loadBoxes);
  const ViewIcon =
    view.kind === "role"
      ? { sent: Send, drafts: FilePen, junk: ShieldAlert, trash: Trash2 }[view.role]
      : view.kind === "folder"
        ? Folder
        : view.kind === "archive"
          ? Archive
          : Inbox;

  useEffect(() => {
    // Folder lists up front, so the menu opens at once.
    if (IN_TAURI) for (const a of accounts) void loadBoxes(a.id).catch(() => null);
  }, [accounts, loadBoxes]);

  const open = async () => {
    const { open: show } = useMailbox.getState();
    const boxes = await Promise.all(
      accounts.map((a) => loadBoxes(a.id).catch((): Mailbox[] => [])),
    );
    const same = (v: View) => JSON.stringify(v) === JSON.stringify(view);
    const item = (v: View) => ({
      text: viewLabel(v, t),
      checked: same(v),
      action: () => void show(v),
    });
    const roles = BROWSE_ROLES.filter((role) =>
      boxes.some((list) => list.some((b) => b.role === role)),
    );
    const folders = (account: Account, list: Mailbox[]) =>
      list
        .filter((b) => b.role === "other")
        .sort((a, b) => a.label.localeCompare(b.label))
        .map((b) =>
          item({ kind: "folder", accountId: account.id, folder: b.name, label: mailboxLabel(b) }),
        );
    const perAccount = accounts
      .map((a, i) => ({ account: a, items: folders(a, boxes[i]) }))
      .filter((x) => x.items.length > 0);
    await popupMenu([
      item({ kind: "inbox" }),
      item({ kind: "archive" }),
      ...(roles.length ? [{ item: "Separator" as const }] : []),
      ...roles.map((role) => item({ kind: "role", role })),
      ...(perAccount.length ? [{ item: "Separator" as const }] : []),
      ...(perAccount.length === 1
        ? [{ text: t("mailbox.folders"), items: perAccount[0].items }]
        : perAccount.map((x) => ({ text: x.account.label, items: x.items }))),
    ]);
  };

  return (
    <button
      type="button"
      aria-label={t("mailbox.choose")}
      title={t("mailbox.choose")}
      onClick={() => void open().catch((e: Error) => useMail.getState().notify(e.message))}
      className="mr-auto flex h-7 min-w-0 items-center gap-1 rounded-md px-2 text-[13px] font-medium text-text hover:bg-hover"
    >
      <ViewIcon size={14} aria-hidden className="shrink-0 text-text-2" />
      <span className="truncate">{viewLabel(view, t)}</span>
      <ChevronDown size={13} aria-hidden className="shrink-0 text-text-2" />
    </button>
  );
}

/** End of a browsed mailbox: the next page from the server, on click. */
function BrowseMoreRow() {
  const { t } = useTranslation();
  const { loading, error, more, hasMore } = useMailbox(
    useShallow((m) => ({
      loading: m.loading,
      error: m.error,
      more: m.more,
      hasMore: m.sources.some((src) => !src.started || src.before !== null),
    })),
  );
  return (
    <li className="flex flex-col items-center gap-1 px-3 py-3 text-center text-[12px]">
      {loading ? (
        <span className="flex items-center gap-1.5 text-text-2" role="status">
          <RotateCw size={12} className="animate-spin" aria-hidden />
          {t("older.loading")}
        </span>
      ) : (
        <>
          {error && <span className="text-text-2">{t("mailbox.failed", { error })}</span>}
          {hasMore || error ? (
            <button
              type="button"
              // A failed start (folder list) has nothing to continue: open it again.
              onClick={() => {
                const m = useMailbox.getState();
                void (m.sources.length ? more() : m.open(m.view));
              }}
              className="rounded-md px-2 py-1 text-accent-text hover:bg-hover"
            >
              {t(error ? "mailbox.retry" : "older.load")}
            </button>
          ) : (
            <span className="text-text-3">{t("older.none")}</span>
          )}
        </>
      )}
    </li>
  );
}

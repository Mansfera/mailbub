import {
  Fragment,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import {
  Archive,
  Bell,
  BellOff,
  Check,
  ChevronLeft,
  ChevronsUpDown,
  CircleAlert,
  Clock3,
  Ellipsis,
  FolderInput,
  Inbox,
  MailWarning,
  Pin,
  PinOff,
  Reply,
  RotateCw,
  SendHorizontal,
  Trash2,
  X,
} from "lucide-react";
import { popupMenu } from "../lib/popupMenu";
import { useMail, usePrefs } from "../store";
import { useDraftFiles, useFileDrop } from "../lib/draftFiles";
import { AttachButton, DraftFileList, DropOverlay } from "./DraftFiles";
import { IN_TAURI } from "../lib/core";
import { openChatMenu } from "../lib/chatMenu";
import { openMessageMenu } from "../lib/messageMenu";
import { openSettingsWindow } from "../lib/settingsWindow";
import {
  accountAddresses,
  chatDisplayName,
  effectiveStatus,
  isNoReplyChat,
  normalizeSubject,
  replyFrom,
} from "../lib/chat";
import { formatDayHeading, formatTime, isSameDay } from "../lib/format";
import { AccountDot, Avatar, IconButton, Kbd } from "./primitives";
import { ProviderList } from "./AddAccountDialog";
import { HtmlBody } from "./HtmlBody";
import { RemoteBanner } from "./RemoteBanner";
import { Suggestions } from "./Suggestions";
import { UnsubscribeBanner } from "./UnsubscribeBanner";
import { isBrowsedChat, moveMenuItems, selectedAccountId, useMailbox } from "../lib/mailbox";
import { AttachmentChip } from "./AttachmentChip";
import { blockedDomains, decidePolicy, prepareHtml } from "../lib/emailHtml";
import { linkify } from "../lib/linkify";
import { useCssColor } from "../lib/useCssColor";
import type { Account, Chat, ChatStatus, Message } from "../types";
import type { TFunction } from "i18next";

const EMPTY: Message[] = [];
const STATUSES: ChatStatus[] = ["needs_reply", "waiting", "done"];
const GROUP_GAP_MS = 5 * 60_000;
const NO_DROP_TARGET = { current: null };

/** A no-reply chat, unless Settings says to treat it like any other. */
function useReadOnly(chat: Chat): boolean {
  const pref = usePrefs((s) => s.noReplyReadOnly);
  return pref && isNoReplyChat(chat);
}

export function Conversation() {
  const { t } = useTranslation();
  const selectedId = useMail((s) => s.selectedId);
  const local = useMail((s) => s.chats.find((c) => c.id === s.selectedId));
  // A chat of a mailbox read live from the server (Trash, Junk, …): read-only here.
  const browsed = useMailbox((s) => s.chats.find((c) => c.id === selectedId));
  const chat = local ?? browsed;
  const account = useMail((s) => s.accounts.find((a) => a.id === chat?.accountId));
  const hasAccounts = useMail((s) => s.accounts.length > 0);
  const accountsLoaded = useMail((s) => s.accountsLoaded);
  const hasChats = useMail((s) => s.chats.length > 0);
  const syncing = useMail((s) => s.sync.running);
  const empty = hasChats ? "empty" : syncing ? "syncing" : "noChats";
  const pane = useRef<HTMLElement>(null);
  const accountId = chat?.accountId;
  useEffect(() => {
    // Folder list for "Move to", so the menu opens without waiting for the server.
    if (accountId && IN_TAURI)
      void useMailbox
        .getState()
        .loadBoxes(accountId)
        .catch(() => null);
  }, [accountId]);

  if (!chat || !account) {
    return (
      <main
        className={`flex flex-1 flex-col bg-bg ${
          accountsLoaded && !hasAccounts ? "" : "max-[700px]:hidden"
        }`}
      >
        <div data-tauri-drag-region className="h-[52px] shrink-0" />
        {!accountsLoaded ? null : !hasAccounts ? (
          <Welcome />
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 pb-[52px] text-center">
            <h1 className="text-[22px] font-semibold tracking-tight">
              {t(`conversation.${empty}Title`)}
            </h1>
            <p className="max-w-xs text-text-2">{t(`conversation.${empty}Hint`)}</p>
          </div>
        )}
      </main>
    );
  }

  return (
    <main ref={pane} className="relative flex min-w-0 flex-1 flex-col bg-bg">
      <Header chat={chat} account={account} />
      {/* Keys must differ from the Composer's: siblings sharing one get duplicated. */}
      {!browsed && <UnsubscribeBanner key={`unsubscribe:${chat.id}`} chat={chat} />}
      <MessageList chat={chat} />
      {!browsed && <Composer key={chat.id} chat={chat} account={account} dropTarget={pane} />}
    </main>
  );
}

/** First run: no accounts yet. */
function Welcome() {
  const { t } = useTranslation();
  const setSetup = useMail((s) => s.setSetup);
  return (
    <div className="flex flex-1 items-start justify-center overflow-y-auto px-6 pt-[12vh] pb-10">
      <div className="w-full max-w-[440px]">
        <h1 className="text-[26px] leading-tight font-semibold tracking-tight">
          {t("setup.chooseTitle")}
        </h1>
        <p className="mt-2 mb-5 text-text-2">{t("setup.chooseHint")}</p>
        <div className="-mx-3">
          <ProviderList onPick={setSetup} />
        </div>
      </div>
    </div>
  );
}

function Header({ chat, account }: { chat: Chat; account: Account }) {
  const { t } = useTranslation();
  const { togglePin, toggleMute, toggleArchive, markUnread, deleteChat } = useMail.getState();
  const browsed = isBrowsedChat(chat.id);
  const browsedMessages = useMailbox((s) => s.messages[chat.id]);
  const subtitle =
    chat.participants.length === 1
      ? chat.participants[0].address
      : t("chat.participants", { count: chat.participants.length + 1 });

  return (
    <header
      data-tauri-drag-region
      className="flex h-[52px] shrink-0 items-center gap-3 border-b border-line px-4 max-[700px]:gap-2 max-[700px]:pl-1.5"
    >
      <IconButton
        label={t("setup.back")}
        className="min-[701px]:hidden"
        onClick={() => useMail.getState().select(null)}
      >
        <ChevronLeft size={20} />
      </IconButton>
      <Avatar people={chat.participants} size={30} />
      <div className="min-w-0 flex-1" data-tauri-drag-region>
        <div className="truncate text-[15px] leading-5 font-semibold">{chatDisplayName(chat)}</div>
        <div
          className="flex items-center gap-1.5 truncate text-[11px] text-text-2"
          title={chat.participants.map((p) => p.address).join(", ")}
        >
          <span className="truncate">{subtitle}</span>
          <span aria-hidden>·</span>
          <AccountDot color={account.color} />
          <span className="shrink-0">{t("conversation.via", { account: account.label })}</span>
        </div>
      </div>
      {browsed ? (
        <IconButton
          label={t("message.moveTo")}
          onClick={() =>
            void moveMenuItems(chat.accountId, browsedMessages ?? [])
              .then(popupMenu)
              .catch((e: Error) => useMail.getState().notify(e.message))
          }
        >
          <FolderInput size={16} />
        </IconButton>
      ) : (
        <>
          {/* No room on a phone: the status and actions move into the chat's menu. */}
          <div className="max-[700px]:hidden">
            <StatusPicker chat={chat} />
          </div>
          <IconButton
            label={t("conversation.actions")}
            className="min-[701px]:hidden"
            onClick={() => void openChatMenu(chat, t)}
          >
            <Ellipsis size={18} />
          </IconButton>
        </>
      )}
      <div hidden={browsed} className="flex items-center gap-0.5 max-[700px]:hidden">
        <IconButton
          label={t(chat.pinned ? "conversation.unpin" : "conversation.pin")}
          active={chat.pinned}
          onClick={() => togglePin(chat.id)}
        >
          {chat.pinned ? <PinOff size={16} /> : <Pin size={16} />}
        </IconButton>
        <IconButton
          label={t(chat.muted ? "conversation.unmute" : "conversation.mute")}
          active={chat.muted}
          onClick={() => toggleMute(chat.id)}
        >
          {chat.muted ? <Bell size={16} /> : <BellOff size={16} />}
        </IconButton>
        <IconButton label={t("conversation.markUnread")} onClick={() => markUnread(chat.id)}>
          <MailWarning size={16} />
        </IconButton>
        <IconButton
          label={t(chat.archived ? "conversation.unarchive" : "conversation.archive")}
          onClick={() => toggleArchive(chat.id)}
        >
          {chat.archived ? <Inbox size={16} /> : <Archive size={16} />}
        </IconButton>
        <IconButton label={`${t("conversation.delete")} (⌫)`} onClick={() => deleteChat(chat.id)}>
          <Trash2 size={16} />
        </IconButton>
      </div>
    </header>
  );
}

function StatusPicker({ chat }: { chat: Chat }) {
  const { t } = useTranslation();
  const setStatus = useMail((s) => s.setStatus);
  const current = effectiveStatus(chat);
  const manual = Boolean(chat.manualStatus);
  return (
    <div
      role="radiogroup"
      aria-label={t("sidebar.statusFilter")}
      title={manual ? undefined : t("status.suggested")}
      className="flex rounded-md bg-surface p-0.5"
    >
      {STATUSES.map((st) => {
        const on = current === st;
        return (
          <button
            key={st}
            role="radio"
            aria-checked={on}
            onClick={() => setStatus(chat.id, st)}
            className={`h-6 rounded px-2 text-[12px] whitespace-nowrap ${
              on
                ? `bg-raised font-medium shadow-sm ${st === "done" ? "text-done" : st === "needs_reply" ? "text-accent-text" : "text-text"} ${
                    manual
                      ? ""
                      : "outline outline-1 -outline-offset-1 outline-dashed outline-text-3"
                  }`
                : "text-text-2 hover:text-text"
            }`}
          >
            {t(`status.${st}`)}
          </button>
        );
      })}
    </div>
  );
}

function MessageList({ chat }: { chat: Chat }) {
  const { t } = useTranslation();
  const browsedMessages = useMailbox((s) => s.messages[chat.id]);
  const stored = useMail((s) => s.messages[chat.id]) ?? browsedMessages ?? EMPTY;
  const outbox = useMail((s) => s.outbox);
  const account = useMail((s) => s.accounts.find((a) => a.id === chat.accountId));
  // Replies still in the outbox follow the stored messages, as outgoing bubbles.
  const messages = useMemo(() => {
    const pending = outbox.filter((o) => o.chatId === chat.id);
    if (!pending.length || !account) return stored;
    return [
      ...stored,
      ...pending.map((o): Message => ({
        id: o.id,
        from: {
          name: account.displayName || (o.from ?? account.defaultFrom),
          address: o.from ?? account.defaultFrom,
        },
        outgoing: true,
        sentAt: o.sentAt,
        subject: stored[stored.length - 1]?.subject ?? "",
        body: o.body,
        attachments: (o.files ?? []).map((f, i) => ({ id: String(i), name: f.name, size: f.size })),
        read: true,
        delivery: o.delivery,
        deliveryError: o.error,
        replyTo: o.replyTo,
      })),
    ];
  }, [stored, outbox, chat.id, account]);
  const ref = useRef<HTMLDivElement>(null);
  const group = chat.participants.length > 1;
  const lastId = messages[messages.length - 1]?.id;
  // Where the user is, as distance from the bottom, for the chat and newest message shown.
  const anchor = useRef<{ chatId: string; lastId?: string; fromBottom: number } | null>(null);
  const loadOlder = useMail((s) => s.loadOlder);
  const olderLoading = useMail((s) => s.olderLoading);
  // A browsed mailbox pages in the sidebar; the cache's backfill has nothing to add here.
  const olderDone = useMail((s) => Boolean(s.olderDone[chat.accountId])) || isBrowsedChat(chat.id);
  const unreadFrom = useMail((s) =>
    s.unreadFrom?.chatId === chat.id ? s.unreadFrom.messageId : null,
  );
  const content = useRef<HTMLDivElement>(null);
  const unreadLine = useRef<HTMLDivElement>(null);
  // Opened with unread mail: the "New messages" line is held at the top (while mail frames
  // above and below it size themselves) until the user scrolls.
  const pinned = useRef(false);
  // Landed and not scrolled since: stay put relative to the bottom while mail frames size
  // themselves above, as WebKit has no scroll anchoring to do it.
  const settling = useRef(true);
  const landed = useRef<string | null>(null);
  const byId = useMemo(() => new Map(messages.map((m) => [m.id, m])), [messages]);
  const readOnly = useReadOnly(chat);

  const holdLine = useCallback(() => {
    const el = ref.current;
    const line = unreadLine.current;
    if (!el || !line || !pinned.current) return false;
    // When the new part fits on screen this clamps to the bottom, as without unread mail.
    el.scrollTop =
      line.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop - 12;
    anchor.current = { chatId: chat.id, lastId, fromBottom: el.scrollHeight - el.scrollTop };
    return true;
  }, [chat.id, lastId]);

  // Land at the newest message when switching chats or when a new one arrives (or at the top
  // of what is new, if the chat was opened unread); when older mail is added above (or the
  // list reloads), stay where the user was.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (unreadFrom !== landed.current) {
      landed.current = unreadFrom;
      pinned.current = unreadFrom !== null;
    }
    if (holdLine()) return;
    const a = anchor.current;
    const returning = a && a.chatId === chat.id && a.lastId === lastId;
    if (!returning) settling.current = true;
    el.scrollTop = returning ? el.scrollHeight - a.fromBottom : el.scrollHeight;
    anchor.current = { chatId: chat.id, lastId, fromBottom: el.scrollHeight - el.scrollTop };
  }, [chat.id, messages, lastId, unreadFrom, holdLine]);

  useEffect(() => {
    const el = content.current;
    if (!el) return;
    const observer = new ResizeObserver(() => {
      if (holdLine() || !settling.current) return;
      const scroller = ref.current;
      if (scroller && anchor.current) {
        scroller.scrollTop = scroller.scrollHeight - anchor.current.fromBottom;
      }
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [holdLine]);

  const release = () => {
    pinned.current = false;
    settling.current = false;
  };

  return (
    <div
      ref={ref}
      onWheel={release}
      onTouchMove={release}
      onMouseDown={release}
      onKeyDown={release}
      onScroll={(e) => {
        const el = e.currentTarget;
        // While settling the only scrolling is ours, and a frame may have grown since.
        if (!settling.current) {
          anchor.current = { chatId: chat.id, lastId, fromBottom: el.scrollHeight - el.scrollTop };
        }
        // Scrolled up to the start: fetch the account's older mail (only on a real scroll,
        // so opening a short chat never starts a download).
        if (
          el.scrollTop < 40 &&
          el.scrollHeight > el.clientHeight &&
          !olderDone &&
          !pinned.current &&
          !settling.current
        ) {
          void loadOlder(chat.accountId);
        }
      }}
      className={`flex-1 overflow-y-auto py-4 ${readOnly ? "px-5 max-[700px]:px-2" : "px-5"}`}
    >
      <div ref={content} className="mx-auto flex max-w-[760px] flex-col">
        {messages.length > 0 && !olderDone && (
          <div className="mb-2 flex justify-center text-[12px]">
            {olderLoading ? (
              <span className="flex items-center gap-1.5 text-text-2" role="status">
                <RotateCw size={12} className="animate-spin" aria-hidden />
                {t("older.loading")}
              </span>
            ) : (
              <button
                type="button"
                onClick={() => void loadOlder(chat.accountId)}
                className="rounded-md px-2 py-1 text-accent-text hover:bg-hover"
              >
                {t("older.load")}
              </button>
            )}
          </div>
        )}
        {messages.map((m, i) => {
          const prev = messages[i - 1];
          const newDay = !prev || !isSameDay(prev.sentAt, m.sentAt);
          const newSubject =
            !prev || normalizeSubject(prev.subject) !== normalizeSubject(m.subject);
          // Email replies always cite a parent; quote it only when it is not the message
          // right above, as a messenger does for a reply to an earlier message.
          const parent = m.replyTo && m.replyTo !== prev?.id ? byId.get(m.replyTo) : undefined;
          const continued =
            !parent &&
            prev &&
            !newDay &&
            !newSubject &&
            prev.from.address === m.from.address &&
            Date.parse(m.sentAt) - Date.parse(prev.sentAt) < GROUP_GAP_MS;
          return (
            <Fragment key={m.id}>
              {m.id === unreadFrom && (
                <div
                  ref={unreadLine}
                  role="separator"
                  aria-label={t("conversation.newMessages")}
                  className="my-3 flex items-center gap-3 text-[11px] font-medium text-accent-text"
                >
                  <span className="h-px flex-1 bg-current opacity-30" />
                  {t("conversation.newMessages")}
                  <span className="h-px flex-1 bg-current opacity-30" />
                </div>
              )}
              {newDay && (
                <div className="my-3 text-center text-[11px] font-medium text-text-2">
                  {formatDayHeading(m.sentAt)}
                </div>
              )}
              {newSubject && (
                <div className="my-2 flex justify-center">
                  <span
                    className="max-w-[80%] truncate rounded-full border border-line px-3 py-0.5 text-[11px] text-text-2"
                    title={m.subject}
                  >
                    {normalizeSubject(m.subject) || "—"}
                  </span>
                </div>
              )}
              <Bubble
                message={m}
                chatId={chat.id}
                parent={parent}
                readOnly={readOnly}
                showSender={group && !m.outgoing && !continued}
                continued={Boolean(continued)}
              />
            </Fragment>
          );
        })}
      </div>
    </div>
  );
}

/** Short text standing for a message in reply quotes. */
function replySnippet(m: Message, t: TFunction): string {
  const line = m.body.split("\n").find((l) => l.trim());
  return line?.trim() || m.attachments[0]?.name || m.subject || t("chat.attachment");
}

/** Scrolls a message into view and flashes it. */
function jumpTo(messageId: string) {
  const el = document.querySelector<HTMLElement>(`[data-message="${messageId}"]`);
  if (!el) return;
  el.scrollIntoView({ block: "center", behavior: "smooth" });
  el.animate([{ boxShadow: "0 0 0 3px var(--brand)" }, { boxShadow: "0 0 0 3px transparent" }], {
    duration: 1400,
    easing: "ease-out",
  });
}

/** Starts a reply to one message in the composer. */
function replyTo(chatId: string, messageId: string) {
  useMail.getState().setReplyTarget({ chatId, messageId });
  document.getElementById("composer")?.focus();
}

function ReplyQuote({ parent, onAccent }: { parent: Message; onAccent: boolean }) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      onClick={() => jumpTo(parent.id)}
      className={`mb-1.5 flex w-full min-w-0 flex-col rounded-md border-l-[3px] py-0.5 pr-2 pl-2 text-left text-[13px] leading-[1.35] ${
        onAccent
          ? "border-white/70 bg-white/12 hover:bg-white/18"
          : "border-brand bg-brand/8 hover:bg-brand/12"
      }`}
    >
      <span className={`truncate font-semibold ${onAccent ? "text-white" : "text-accent-text"}`}>
        {parent.outgoing ? t("chat.you") : parent.from.name}
      </span>
      <span className={`truncate ${onAccent ? "text-white/85" : "text-text-2"}`}>
        {replySnippet(parent, t)}
      </span>
    </button>
  );
}

function Bubble({
  message: m,
  chatId,
  parent,
  readOnly,
  showSender,
  continued,
}: {
  message: Message;
  chatId: string;
  /** The message this one replies to, when it is worth quoting. */
  parent?: Message;
  /** In a no-reply chat: nothing to reply to, and what came in takes the full width. */
  readOnly: boolean;
  showSender: boolean;
  continued: boolean;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const out = m.outgoing;
  const wide = readOnly && !out;
  const rules = useMail((s) => s.remoteRules);
  const allowedOnce = useMail((s) => Boolean(s.remoteAllowedOnce[m.id]));
  const readablePref = usePrefs((s) => s.readable);
  // Per-message "Show original" / "Show readable version" from the context menu.
  const [override, setOverride] = useState<boolean | null>(null);
  const original = override ?? !readablePref;
  const scheme = useCssColor("color-scheme") === "dark" ? "dark" : "light";
  const textColor = useCssColor("--text");
  const linkColor = useCssColor("--accent-text");
  const colors = useMemo(
    () => (out ? { text: "#ffffff", link: "#ffffff" } : { text: textColor, link: linkColor }),
    [out, textColor, linkColor],
  );
  const prepared = useMemo(
    () =>
      m.html
        ? prepareHtml(m.html, {
            policy: (domains) => decidePolicy(domains, rules, m.from.address, allowedOnce),
            stripQuotes: !expanded,
            readable: !original,
            colors,
            scheme,
          })
        : null,
    [m.html, m.from.address, rules, allowedOnce, expanded, original, colors, scheme],
  );
  const blocked = prepared ? blockedDomains(prepared.remoteDomains, prepared.policy) : [];
  const isCard = Boolean(prepared && prepared.display !== "bubble");
  const canExpand = prepared ? prepared.hasQuote : Boolean(m.quoted);
  // Not on a reply still in the outbox, nor in a read-only browsed mailbox.
  const canReply = !m.delivery && !isBrowsedChat(chatId) && !readOnly;
  const openMenu = (selection?: string) => {
    const accountId = useMailbox.getState().refs[m.id]?.accountId ?? selectedAccountId();
    // Without the folder list (offline, no core) the menu opens without "Move to".
    const moveTo = accountId ? moveMenuItems(accountId, [m]).catch(() => null) : null;
    void Promise.resolve(moveTo).then((items) =>
      openMessageMenu(m, t, {
        reply: canReply ? () => replyTo(chatId, m.id) : null,
        quote: canExpand ? { expanded, toggle: () => setExpanded(!expanded) } : null,
        view: m.html ? { original, toggle: () => setOverride(!original) } : null,
        selection,
        moveTo: items,
      }),
    );
  };
  const onContextMenu = (e: MouseEvent) => {
    if (!IN_TAURI) return;
    e.preventDefault();
    // Outbox replies are not in the database yet, so the message menu has nothing to act on.
    if (!m.delivery) openMenu();
  };
  const linkCls = out ? "underline" : "text-accent-text underline";

  const attachments = m.attachments.length > 0 && (
    <div className="mt-2 flex flex-wrap gap-1.5">
      {m.attachments.map((a) => (
        <AttachmentChip
          key={a.id}
          messageId={m.id}
          attachment={a}
          onAccent={out && !isCard}
          // An outbox reply's files are only on this device until it is sent; mail browsed
          // live from the server is not in the cache the attachment is opened from.
          disabled={Boolean(m.delivery) || m.id.startsWith("box|")}
        />
      ))}
    </div>
  );

  const time = (light: boolean) => (
    <div
      className={`mt-0.5 flex items-center justify-end gap-1 text-[11px] tabular-nums ${light ? "text-white/75" : "text-text-2"}`}
    >
      {formatTime(m.sentAt)}
      {out && <Receipt delivery={m.delivery} />}
    </div>
  );

  const replyButton = canReply && (
    <button
      type="button"
      aria-label={t("message.reply")}
      title={t("message.reply")}
      onClick={() => replyTo(chatId, m.id)}
      className={`flex size-7 shrink-0 items-center justify-center self-center rounded-full text-text-2 opacity-0 group-hover:opacity-100 hover:bg-hover hover:text-text focus-visible:opacity-100 ${
        out ? "order-first mr-1" : "ml-1"
      }`}
    >
      <Reply size={15} />
    </button>
  );

  const suggestions = m.delivery ? (
    m.delivery === "failed" && <FailedNotice message={m} />
  ) : (
    <div className={`flex ${out ? "justify-end" : "justify-start"}`}>
      <Suggestions message={m} align={out ? "end" : "start"} />
    </div>
  );

  // Newsletter-style layouts (and any message shown as original): a card wider than a bubble.
  if (prepared && isCard) {
    return (
      <>
        <div
          className={`group flex ${out ? "justify-end" : "justify-start"} ${continued ? "mt-0.5" : "mt-2.5"}`}
        >
          <div className={`flex flex-col gap-1.5 ${wide ? "w-full" : "w-[min(680px,92%)]"}`}>
            {showSender && (
              <div className="text-[12px] font-semibold text-accent-text">{m.from.name}</div>
            )}
            {parent && <ReplyQuote parent={parent} onAccent={false} />}
            {blocked.length > 0 && <RemoteBanner message={m} blocked={blocked} />}
            <div
              data-message={m.id}
              onContextMenu={onContextMenu}
              className={`overflow-hidden rounded-2xl border border-line ${
                prepared.display === "original" ? "bg-white" : "bg-surface"
              }`}
            >
              <HtmlBody
                srcdoc={prepared.srcdoc}
                sizeKey={`${m.id}|${prepared.display}|${expanded}|${wide}`}
                sizeHints={prepared.sizeHints}
                colorScheme={prepared.display === "original" ? "light" : scheme}
                wide={wide}
                onContextMenu={IN_TAURI ? openMenu : undefined}
              />
            </div>
            <div className="flex items-start justify-between gap-2 px-1">
              <div>{attachments}</div>
              {time(false)}
            </div>
          </div>
          {replyButton}
        </div>
        {suggestions}
      </>
    );
  }

  return (
    <>
      <div
        className={`group flex ${out ? "justify-end" : "justify-start"} ${continued ? "mt-0.5" : "mt-2.5"}`}
      >
        <div
          data-message={m.id}
          onContextMenu={onContextMenu}
          className={`selectable rounded-2xl px-3 py-2 text-[14px] leading-[1.4] ${
            wide ? "w-full" : "max-w-[min(560px,78%)]"
          } ${
            out
              ? "rounded-br-md bg-accent text-on-accent"
              : `bg-surface text-text ${wide ? "" : "rounded-bl-md"}`
          }`}
        >
          {showSender && (
            <div className="mb-0.5 text-[12px] font-semibold text-accent-text">{m.from.name}</div>
          )}
          {parent && <ReplyQuote parent={parent} onAccent={out} />}
          {blocked.length > 0 && (
            <div className="mb-2 text-text">
              <RemoteBanner message={m} blocked={blocked} />
            </div>
          )}
          {prepared ? (
            <HtmlBody
              // Fitting measures the text once, on load: a change of layout needs a new frame.
              key={String(wide)}
              srcdoc={prepared.srcdoc}
              sizeKey={`${m.id}|${prepared.display}|${expanded}|${wide}`}
              sizeHints={prepared.sizeHints}
              colorScheme={scheme}
              fit={!wide}
              wide={wide}
              onContextMenu={IN_TAURI ? openMenu : undefined}
            />
          ) : (
            <>
              <div className="whitespace-pre-wrap [overflow-wrap:anywhere]">
                {linkify(m.body, linkCls)}
              </div>
              {expanded && m.quoted && (
                <div
                  className={`mt-2 border-l-2 pl-2 text-[13px] whitespace-pre-wrap [overflow-wrap:anywhere] ${out ? "border-white/40 text-white/80" : "border-line text-text-2"}`}
                >
                  {linkify(m.quoted, linkCls)}
                </div>
              )}
            </>
          )}
          {attachments}
          {time(out)}
        </div>
        {replyButton}
      </div>
      {suggestions}
    </>
  );
}

/** Delivery receipt after the time of an outgoing message. */
function Receipt({ delivery }: { delivery?: Message["delivery"] }) {
  const { t } = useTranslation();
  const state = delivery ?? "sent";
  const Icon = { sending: Clock3, sent: Check, failed: CircleAlert }[state];
  return (
    <span role="img" aria-label={t(`delivery.${state}`)} title={t(`delivery.${state}`)}>
      <Icon size={12} strokeWidth={state === "sent" ? 2.5 : 2} aria-hidden />
    </span>
  );
}

/** Under a reply that could not be sent: the reason, Retry, and Edit. */
function FailedNotice({ message: m }: { message: Message }) {
  const { t } = useTranslation();
  const retrySend = useMail((s) => s.retrySend);
  const editFailed = useMail((s) => s.editFailed);
  return (
    <div role="alert" className="mt-1 mb-1.5 flex justify-end">
      <div className="flex max-w-[min(560px,78%)] items-start gap-1.5 text-[12px]">
        <CircleAlert size={14} className="mt-px shrink-0 text-account-red" aria-hidden />
        <div className="min-w-0">
          <span className="font-medium text-text">{t("delivery.failed")}</span>
          {m.deliveryError && (
            <span
              className="selectable text-text-2 [overflow-wrap:anywhere]"
              title={m.deliveryError}
            >
              {" · "}
              {m.deliveryError}
            </span>
          )}
        </div>
        <button
          type="button"
          onClick={() => retrySend(m.id)}
          className="-my-0.5 shrink-0 rounded px-1.5 py-0.5 font-medium text-accent-text hover:bg-hover"
        >
          {t("delivery.retry")}
        </button>
        <button
          type="button"
          onClick={() => editFailed(m.id)}
          className="-my-0.5 shrink-0 rounded px-1.5 py-0.5 text-accent-text hover:bg-hover"
        >
          {t("delivery.edit")}
        </button>
      </div>
    </div>
  );
}

function Composer({
  chat,
  account,
  dropTarget,
}: {
  chat: Chat;
  account: Account;
  /** Files dropped anywhere on it are attached to the reply. */
  dropTarget: RefObject<HTMLElement | null>;
}) {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const files = useDraftFiles();
  const addFiles = files.add;
  const readOnly = useReadOnly(chat);
  // A draft keeps the box open: "Edit" on a failed reply brings its text back here.
  const hidden = readOnly && !text && !files.files.length;
  const dragging = useFileDrop(hidden ? NO_DROP_TARGET : dropTarget, addFiles);
  const sendReply = useMail((s) => s.sendReply);
  const setReplyTarget = useMail((s) => s.setReplyTarget);
  const target = useMail((s) =>
    s.replyTarget?.chatId === chat.id
      ? s.messages[chat.id]?.find((m) => m.id === s.replyTarget?.messageId)
      : undefined,
  );
  const stored = useMail((s) => s.messages[chat.id]);
  const ref = useRef<HTMLTextAreaElement>(null);
  const canSend = (text.trim().length > 0 || files.files.length > 0) && IN_TAURI;
  const addresses = accountAddresses(account);
  // A pick lasts while this chat is open; otherwise the conversation decides.
  const [picked, setPicked] = useState<string | null>(null);
  const from =
    picked && addresses.includes(picked)
      ? picked
      : replyFrom(account, stored ?? [chat.last], target);

  // The reply goes to the outbox and shows as a bubble with a receipt; the box is free at once.
  const send = () => {
    if (!canSend) return;
    sendReply(chat.id, text, files.files, target?.id, from);
    setText("");
    files.clear();
    setReplyTarget(null);
  };

  // Edit on a failed reply: its text comes back here, ahead of anything typed since.
  useEffect(
    () =>
      useMail.subscribe((s) => {
        const fill = s.composerFill;
        if (fill?.chatId !== chat.id) return;
        s.clearComposerFill();
        setText((prev) => (prev.trim() ? `${fill.text}\n\n${prev}` : fill.text));
        if (fill.files.length) addFiles(fill.files);
        if (fill.replyTo) s.setReplyTarget({ chatId: chat.id, messageId: fill.replyTo });
        if (fill.from) setPicked(fill.from);
        ref.current?.focus();
      }),
    [chat.id, addFiles],
  );

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [text]);

  if (hidden) {
    return (
      <div className="flex shrink-0 flex-wrap items-center justify-center gap-x-1.5 border-t border-line px-4 py-2 text-[12px] text-text-2">
        <span>{t("composer.noReply")}</span>
        <button
          type="button"
          onClick={() => void openSettingsWindow("mail")}
          className="-my-1 rounded px-1 py-1 text-accent-text hover:bg-hover"
        >
          {t("composer.noReplySettings")}
        </button>
      </div>
    );
  }

  return (
    <div className="shrink-0 border-t border-line px-4 pt-2 pb-3">
      <div className="mx-auto max-w-[760px]">
        <div className="mb-1.5 flex items-center gap-1.5 text-[11px] text-text-2">
          <AccountDot color={account.color} />
          {addresses.length > 1 ? (
            // The native menu sits invisibly over the text, so the line reads as plain text.
            <label className="relative flex min-w-0 items-center gap-1 rounded-sm focus-within:text-text hover:text-text">
              <span className="truncate">{t("composer.from", { address: from })}</span>
              <ChevronsUpDown size={11} className="shrink-0" aria-hidden />
              <select
                aria-label={t("compose.from")}
                value={from}
                onChange={(e) => setPicked(e.target.value)}
                className="absolute inset-0 cursor-default appearance-none opacity-0"
              >
                {addresses.map((a) => (
                  <option key={a} value={a}>
                    {a}
                  </option>
                ))}
              </select>
            </label>
          ) : (
            t("composer.from", { address: from })
          )}
        </div>
        <div className="rounded-xl border border-line bg-bg py-2 pr-3 pl-1.5 focus-within:border-brand">
          {target && (
            <div className="mb-2 flex items-center gap-2 pl-1.5">
              <Reply size={16} className="shrink-0 text-accent-text" aria-hidden />
              <button
                type="button"
                onClick={() => jumpTo(target.id)}
                className="min-w-0 flex-1 border-l-[3px] border-brand pl-2 text-left text-[12px] leading-[1.35]"
              >
                <div className="truncate font-semibold text-accent-text">
                  {t("composer.replyingTo", {
                    name: target.outgoing ? t("chat.you") : target.from.name,
                  })}
                </div>
                <div className="truncate text-text-2">{replySnippet(target, t)}</div>
              </button>
              <IconButton
                label={`${t("composer.cancelReply")} (Esc)`}
                onClick={() => {
                  setReplyTarget(null);
                  ref.current?.focus();
                }}
              >
                <X size={15} />
              </IconButton>
            </div>
          )}
          <DraftFileList
            files={files.files}
            onRemove={files.remove}
            className="mb-2 max-h-24 overflow-y-auto pl-1.5"
          />
          <div className="flex items-end gap-2">
            <span className="-my-0.5">
              <AttachButton onFiles={addFiles} />
            </span>
            <textarea
              id="composer"
              ref={ref}
              rows={1}
              value={text}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  if (target) setReplyTarget(null);
                  else e.currentTarget.blur();
                }
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  send();
                }
              }}
              placeholder={t("composer.placeholder", { name: chatDisplayName(chat) })}
              className="selectable max-h-[200px] min-h-6 flex-1 resize-none bg-transparent text-[14px] leading-6 outline-none placeholder:text-text-3"
            />
            <span className="hidden pb-0.5 sm:inline">
              <Kbd>⌘↵</Kbd>
            </span>
            <button
              type="button"
              disabled={!canSend}
              onClick={send}
              aria-label={t("composer.send")}
              title={t("composer.sendHint")}
              className="flex size-7 shrink-0 items-center justify-center rounded-full bg-accent text-on-accent hover:bg-accent-hover disabled:opacity-35"
            >
              <SendHorizontal size={15} />
            </button>
          </div>
        </div>
      </div>
      <DropOverlay show={dragging} />
    </div>
  );
}

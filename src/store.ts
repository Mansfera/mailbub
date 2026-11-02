import { create } from "zustand";
import { persist } from "zustand/middleware";
import { effectiveStatus, isUnsubscribeChat, lastMessage } from "./lib/chat";
import {
  fetchSearch,
  matchesSearch,
  SEARCH_DEBOUNCE_MS,
  searchOrder,
  type SearchResults,
} from "./lib/search";
import { emit } from "@tauri-apps/api/event";
import { core, IN_TAURI, type ChatStateInput, type NewMessage } from "./lib/core";
import { toUploads } from "./lib/draftFiles";
import i18n, { detectLanguage, type Language } from "./i18n";
import type {
  Account,
  Chat,
  ChatStatus,
  Delivery,
  Message,
  Provider,
  RemoteRule,
  SyncStatus,
} from "./types";

export type Theme = "system" | "light" | "dark";
export type StatusFilter = ChatStatus | "all";
/** Tells the other window (main ↔ Settings) to reload accounts or remote-content rules. */
function broadcast(event: "accounts-changed" | "rules-changed") {
  if (IN_TAURI) void emit(event).catch(() => undefined);
}

export type SetupStep = Provider | "choose" | "import";

/** One reversible user action, newest last. */
interface UndoEntry {
  undo: () => void;
  /** After this time (ms) the action has reached the server and can no longer be undone. */
  expiresAt?: number;
}

/** Bottom notice after archive/delete; `undoable` shows an Undo button. */
export interface Toast {
  id: number;
  /** i18n key */
  message: string;
  undoable: boolean;
}

const DELETE_UNDO_MS = 10_000;
/** Failed replies, kept across restarts (read while the store is created, so defined up here). */
const OUTBOX_KEY = "mailbub.outbox";
let outboxSeq = 0;

export type MapsApp = "apple" | "google";
export type CalendarApp = "apple" | "google";

interface PrefsState {
  theme: Theme;
  language: Language;
  /** Where place links open; previews always use free OpenStreetMap. */
  maps: MapsApp;
  calendar: CalendarApp;
  /** Show chats that are only one-click unsubscribe requests (hidden by default). */
  showUnsubscribe: boolean;
  /** Show HTML mail in the app's colours and font (per-message "Show original" overrides). */
  readable: boolean;
  /** No-reply chats: no reply box, a note in its place, and their mail at full width. */
  noReplyReadOnly: boolean;
  /** Load profile pictures (Gravatar, brand logos) for people we write with. */
  avatars: boolean;
  /** Chats whose "Unsubscribe?" banner was dismissed or acted on. */
  unsubscribeHidden: string[];
  setTheme: (theme: Theme) => void;
  setLanguage: (language: Language) => void;
  setMaps: (maps: MapsApp) => void;
  setCalendar: (calendar: CalendarApp) => void;
  setShowUnsubscribe: (show: boolean) => void;
  setReadable: (readable: boolean) => void;
  setNoReplyReadOnly: (noReplyReadOnly: boolean) => void;
  setAvatars: (avatars: boolean) => void;
  hideUnsubscribe: (chatId: string) => void;
}

export const usePrefs = create<PrefsState>()(
  persist(
    (set) => ({
      theme: "system",
      language: detectLanguage(),
      maps: "apple",
      calendar: "apple",
      showUnsubscribe: false,
      readable: true,
      noReplyReadOnly: true,
      avatars: true,
      unsubscribeHidden: [],
      setTheme: (theme) => set({ theme }),
      setLanguage: (language) => set({ language }),
      setMaps: (maps) => set({ maps }),
      setCalendar: (calendar) => set({ calendar }),
      setShowUnsubscribe: (showUnsubscribe) => set({ showUnsubscribe }),
      setReadable: (readable) => set({ readable }),
      setNoReplyReadOnly: (noReplyReadOnly) => set({ noReplyReadOnly }),
      setAvatars: (avatars) => set({ avatars }),
      hideUnsubscribe: (chatId) =>
        set((p) =>
          p.unsubscribeHidden.includes(chatId)
            ? p
            : { unsubscribeHidden: [...p.unsubscribeHidden, chatId] },
        ),
    }),
    { name: "mailbub.prefs" },
  ),
);

interface MailState {
  accounts: Account[];
  accountsLoaded: boolean;
  /** Open add-account dialog: null = closed, "choose" = provider picker, "import" = Apple Mail list. */
  setup: SetupStep | null;
  /** Id of the account whose settings dialog is open. */
  editingAccount: string | null;
  chats: Chat[];
  /** Full message lists of opened chats, by chat id. */
  messages: Record<string, Message[]>;
  sync: SyncStatus;
  selectedId: string | null;
  /**
   * First message that was unread when the open chat was opened: the "New messages" line
   * sits above it until the chat is left or answered.
   */
  unreadFrom: { chatId: string; messageId: string } | null;
  accountFilter: string;
  statusFilter: StatusFilter;
  showArchived: boolean;
  query: string;
  /** Full-text results for `query` (arrive shortly after typing stops). */
  search: SearchResults | null;
  /** Older mail is being fetched from the server. */
  olderLoading: boolean;
  /** Accounts with no older mail left on the server. */
  olderDone: Record<string, true>;
  switcherOpen: boolean;
  composeOpen: boolean;
  /** The message the composer replies to, when the user picked one. */
  replyTarget: { chatId: string; messageId: string } | null;

  select: (id: string | null) => void;
  setAccountFilter: (id: string) => void;
  setStatusFilter: (status: StatusFilter) => void;
  setShowArchived: (show: boolean) => void;
  setQuery: (query: string) => void;
  setSwitcherOpen: (open: boolean) => void;
  setComposeOpen: (open: boolean) => void;
  setReplyTarget: (target: { chatId: string; messageId: string } | null) => void;
  setSetup: (setup: SetupStep | null) => void;
  setEditingAccount: (id: string | null) => void;

  loadAccounts: () => Promise<void>;
  accountAdded: (account: Account) => void;
  accountUpdated: (account: Account) => void;
  removeAccount: (id: string) => Promise<void>;

  loadChats: () => Promise<void>;
  loadMessages: (chatId: string) => Promise<void>;
  setSync: (sync: SyncStatus) => void;
  syncNow: () => void;
  /** Fetches the next batch of older mail: one account, or those the list shows. */
  loadOlder: (accountId?: string) => Promise<void>;

  togglePin: (id: string) => void;
  toggleMute: (id: string) => void;
  /** `toast`: an i18n key to show instead of "Conversation archived". */
  toggleArchive: (id: string, toast?: string) => void;
  setStatus: (id: string, status: ChatStatus) => void;
  markUnread: (id: string) => void;
  markRead: (id: string) => void;
  deleteChat: (id: string) => void;
  /** Replies not yet stored as sent: in flight, or failed and waiting for Retry or Edit. */
  outbox: OutboxItem[];
  /** Text and files going back into a chat's composer (Edit on a failed reply). */
  composerFill: {
    chatId: string;
    text: string;
    files: File[];
    replyTo?: string;
    from?: string;
  } | null;
  /**
   * Queues a reply in the outbox and sends it; the composer can clear at once. `replyTo` is
   * the message it answers, by default the newest.
   */
  sendReply: (
    chatId: string,
    body: string,
    files?: File[],
    replyTo?: string,
    from?: string,
  ) => void;
  retrySend: (id: string) => void;
  /** Takes a failed reply out of the outbox and puts its text and files back in the composer. */
  editFailed: (id: string) => void;
  clearComposerFill: () => void;
  /** Sends a new message and opens its chat; false on failure, like `sendReply`. */
  sendMessage: (draft: NewMessage) => Promise<boolean>;

  remoteRules: RemoteRule[];
  /** Messages whose remote content the user loaded once (this session only). */
  remoteAllowedOnce: Record<string, true>;
  loadRemoteRules: () => Promise<void>;
  addRemoteRules: (rules: RemoteRule[]) => Promise<void>;
  removeRemoteRule: (rule: RemoteRule) => Promise<void>;
  allowRemoteOnce: (messageId: string) => void;

  undoStack: UndoEntry[];
  toast: Toast | null;
  /** ⌘Z: reverts the latest action that can still be reverted. */
  undo: () => void;
  dismissToast: () => void;
  /** Shows a short message (already translated) in the toast. */
  notify: (text: string) => void;
}

function updateChat(chats: Chat[], id: string, fn: (chat: Chat) => Chat): Chat[] {
  return chats.map((c) => (c.id === id ? fn(c) : c));
}

export const useMail = create<MailState>()((set, get) => ({
  accounts: [],
  accountsLoaded: false,
  setup: null,
  editingAccount: null,
  chats: [],
  messages: {},
  undoStack: [],
  toast: null,
  remoteRules: [],
  remoteAllowedOnce: {},
  loadRemoteRules: async () => {
    try {
      set({ remoteRules: await core.listRemoteRules() });
    } catch {
      // No core in the browser preview.
    }
  },
  addRemoteRules: async (rules) => {
    await core.addRemoteRules(rules);
    await get().loadRemoteRules();
    broadcast("rules-changed");
  },
  removeRemoteRule: async (rule) => {
    await core.removeRemoteRule(rule);
    set((s) => ({
      remoteRules: s.remoteRules.filter((r) => !(r.kind === rule.kind && r.value === rule.value)),
    }));
    broadcast("rules-changed");
  },
  allowRemoteOnce: (id) =>
    set((s) => ({ remoteAllowedOnce: { ...s.remoteAllowedOnce, [id]: true } })),
  sync: { running: false, lastRun: null, errors: [] },
  selectedId: null,
  unreadFrom: null,
  accountFilter: "all",
  statusFilter: "all",
  showArchived: false,
  query: "",
  search: null,
  olderLoading: false,
  olderDone: {},
  switcherOpen: false,
  composeOpen: false,
  replyTarget: null,

  select: (id) => {
    const unread = get().chats.find((c) => c.id === id)?.unread ?? 0;
    set((s) => ({
      selectedId: id,
      unreadFrom: null,
      chats: id ? updateChat(s.chats, id, (c) => ({ ...c, unread: 0 })) : s.chats,
    }));
    if (!id) return;
    // Load before marking read, so the list still says which messages are new; the line
    // arrives with the messages in one update, so the view lands on it without a jump.
    void core
      .chatMessages(id)
      .then((list) => {
        const first = unread ? list.find((m) => !m.read && !m.outgoing) : undefined;
        set((s) => ({
          messages: { ...s.messages, [id]: list },
          ...(first && s.selectedId === id && { unreadFrom: { chatId: id, messageId: first.id } }),
        }));
      })
      .catch(() => undefined)
      .finally(() => void core.setChatRead(id, true).catch(() => undefined));
  },
  setAccountFilter: (accountFilter) => set({ accountFilter }),
  setStatusFilter: (statusFilter) => set({ statusFilter }),
  setShowArchived: (showArchived) => set({ showArchived }),
  setQuery: (query) => {
    set({ query });
    scheduleSearch(query);
  },
  setSwitcherOpen: (switcherOpen) => set({ switcherOpen }),
  setComposeOpen: (composeOpen) => set({ composeOpen }),
  setReplyTarget: (replyTarget) => set({ replyTarget }),
  setSetup: (setup) => set({ setup }),
  setEditingAccount: (editingAccount) => set({ editingAccount }),

  loadAccounts: async () => {
    try {
      set({ accounts: await core.listAccounts(), accountsLoaded: true });
    } catch {
      // Outside the desktop shell there is no core; show the empty state.
      set({ accountsLoaded: true });
    }
  },
  accountAdded: (account) => {
    set((s) => ({ accounts: [...s.accounts, account] }));
    get().syncNow();
    broadcast("accounts-changed");
  },
  accountUpdated: (account) => {
    set((s) => ({ accounts: s.accounts.map((a) => (a.id === account.id ? account : a)) }));
    broadcast("accounts-changed");
  },
  loadChats: async () => {
    try {
      const chats = withPendingArchive(await core.listChats());
      set((s) => ({
        chats,
        // Opening a chat marks it read; keep that while the user is looking at it.
        ...(s.selectedId && {
          chats: updateChat(chats, s.selectedId, (c) => ({ ...c, unread: 0 })),
        }),
      }));
      // New mail may match the current search.
      if (get().query.trim()) scheduleSearch(get().query);
    } catch {
      // No core (browser preview) or not ready yet; keep what we have.
    }
  },
  loadMessages: async (chatId) => {
    try {
      const list = await core.chatMessages(chatId);
      set((s) => ({ messages: { ...s.messages, [chatId]: list } }));
    } catch {
      // Leave the chat empty rather than failing the whole view.
    }
  },
  setSync: (sync) => set({ sync }),
  syncNow: () => {
    set((s) => ({ sync: { ...s.sync, running: true } }));
    void core.syncNow().catch(() => set((s) => ({ sync: { ...s.sync, running: false } })));
  },
  loadOlder: async (accountId) => {
    const s = get();
    if (s.olderLoading) return;
    const ids = (
      accountId
        ? [accountId]
        : s.accountFilter === "all"
          ? s.accounts.map((a) => a.id)
          : [s.accountFilter]
    ).filter((id) => !s.olderDone[id]);
    if (!ids.length || !IN_TAURI) return;
    set({ olderLoading: true });
    try {
      const results = await core.loadOlderMail(ids);
      const done = { ...get().olderDone };
      for (const r of results) if (!r.more && !r.error) done[r.accountId] = true;
      set({ olderDone: done });
      const failed = results.find((r) => r.error);
      if (failed) showToast(`${i18n.t("toast.olderFailed")}: ${failed.error}`, false);
      if (results.some((r) => r.added > 0)) {
        await get().loadChats();
        const selected = get().selectedId;
        if (selected) await get().loadMessages(selected);
      }
    } catch (e) {
      showToast(`${i18n.t("toast.olderFailed")}: ${(e as Error).message}`, false);
    } finally {
      set({ olderLoading: false });
    }
  },
  removeAccount: async (id) => {
    await core.removeAccount(id);
    set((s) => ({
      accounts: s.accounts.filter((a) => a.id !== id),
      chats: s.chats.filter((c) => c.accountId !== id),
      accountFilter: s.accountFilter === id ? "all" : s.accountFilter,
      editingAccount: s.editingAccount === id ? null : s.editingAccount,
    }));
    broadcast("accounts-changed");
  },

  togglePin: (id) => {
    patchChat(id, (c) => ({ ...c, pinned: !c.pinned }));
    record(() => patchChat(id, (c) => ({ ...c, pinned: !c.pinned })));
  },
  toggleMute: (id) => {
    patchChat(id, (c) => ({ ...c, muted: !c.muted }));
    record(() => patchChat(id, (c) => ({ ...c, muted: !c.muted })));
  },
  toggleArchive: (id, toast) => {
    const was = get().chats.find((c) => c.id === id);
    if (!was) return;
    const selected = get().selectedId === id;
    setArchived(id, !was.archived);
    set((s) => ({ selectedId: s.selectedId === id ? null : s.selectedId }));
    record(
      () => {
        setArchived(id, was.archived);
        if (selected) set({ selectedId: id });
      },
      { toast: toast ?? (was.archived ? "toast.unarchived" : "toast.archived") },
    );
  },
  setStatus: (id, status) => {
    const prev = get().chats.find((c) => c.id === id)?.manualStatus ?? null;
    patchChat(id, (c) => ({ ...c, manualStatus: status }));
    record(() => patchChat(id, (c) => ({ ...c, manualStatus: prev })));
  },
  markRead: (id) => {
    const prev = get().chats.find((c) => c.id === id)?.unread ?? 0;
    setRead(id, true);
    if (prev) record(() => setRead(id, false, prev));
  },
  markUnread: (id) => {
    set((s) => ({ selectedId: s.selectedId === id ? null : s.selectedId }));
    setRead(id, false);
    record(() => setRead(id, true));
  },
  deleteChat: (id) => {
    const chat = get().chats.find((c) => c.id === id);
    if (!chat) return;
    const selected = get().selectedId === id;
    set((s) => ({
      chats: s.chats.filter((c) => c.id !== id),
      selectedId: selected ? null : s.selectedId,
    }));
    void core.deleteChat(id).catch(() => void get().loadChats());
    record(
      () => {
        void core
          .restoreChat(id)
          .then(() => get().loadChats())
          .then(() => selected && get().select(id));
      },
      { toast: "toast.deleted", ttl: DELETE_UNDO_MS },
    );
  },
  outbox: loadOutbox(),
  composerFill: null,
  sendReply: (chatId, body, files = [], replyTo, from) => {
    const item: OutboxItem = {
      id: `outbox-${Date.now()}-${++outboxSeq}`,
      chatId,
      body,
      ...(files.length && { files }),
      ...(replyTo && { replyTo }),
      ...(from && { from }),
      sentAt: new Date().toISOString(),
      delivery: "sending",
    };
    set((s) => ({
      outbox: [...s.outbox, item],
      // Answering means the new messages were read.
      unreadFrom: s.unreadFrom?.chatId === chatId ? null : s.unreadFrom,
    }));
    void deliver(item);
  },
  retrySend: (id) => {
    const item = get().outbox.find((o) => o.id === id);
    if (!item || item.delivery === "sending") return;
    if (item.lostFiles?.length) {
      // Sending now would leave the files out; let the user attach them again.
      get().editFailed(id);
      showToast(i18n.t("attach.reattach", { names: item.lostFiles.join(", ") }), false);
      return;
    }
    const next: OutboxItem = { ...item, sentAt: new Date().toISOString(), delivery: "sending" };
    delete next.error;
    patchOutbox(id, () => next);
    void deliver(next);
  },
  editFailed: (id) => {
    const item = get().outbox.find((o) => o.id === id);
    if (!item || item.delivery === "sending") return;
    set((s) => ({
      outbox: s.outbox.filter((o) => o.id !== id),
      composerFill: {
        chatId: item.chatId,
        text: item.body,
        files: item.files ?? [],
        replyTo: item.replyTo,
        from: item.from,
      },
    }));
  },
  clearComposerFill: () => set({ composerFill: null }),
  sendMessage: async (draft) => {
    let chatId: string;
    try {
      chatId = await core.sendMessage(draft);
    } catch (e) {
      sendFailed(e);
      return false;
    }
    await get().loadChats();
    set({ composeOpen: false });
    get().select(chatId);
    return true;
  },
  undo: () => {
    const stack = [...get().undoStack];
    let entry = stack.pop();
    let expired = false;
    while (entry?.expiresAt && entry.expiresAt < Date.now()) {
      expired = true;
      entry = stack.pop();
    }
    set({ undoStack: stack });
    if (entry) {
      entry.undo();
      set({ toast: null });
    } else if (expired) {
      showToast("toast.tooLate", false);
    }
  },
  dismissToast: () => set({ toast: null }),
  notify: (text) => showToast(text, false),
}));

let searchTimer: ReturnType<typeof setTimeout> | undefined;

/** Debounced full-text search for the sidebar; results for an outdated query are dropped. */
function scheduleSearch(query: string) {
  clearTimeout(searchTimer);
  if (!query.trim()) {
    useMail.setState({ search: null });
    return;
  }
  searchTimer = setTimeout(() => {
    void fetchSearch(query).then((search) => {
      if (search && useMail.getState().query.trim() === search.query) {
        useMail.setState({ search });
      }
    });
  }, SEARCH_DEBOUNCE_MS);
}

const UNDO_LIMIT = 50;
let toastSeq = 0;

function showToast(message: string, undoable: boolean) {
  toastSeq++;
  useMail.setState({ toast: { id: toastSeq, message, undoable } });
}

/** Remembers how to revert the action just applied; optionally shows an Undo toast. */
function record(undo: () => void, opts: { toast?: string; ttl?: number } = {}) {
  const entry: UndoEntry = { undo, expiresAt: opts.ttl ? Date.now() + opts.ttl : undefined };
  useMail.setState((s) => ({ undoStack: [...s.undoStack, entry].slice(-UNDO_LIMIT) }));
  if (opts.toast) showToast(opts.toast, true);
}

function sendFailed(e: unknown) {
  showToast(`${i18n.t("composer.sendFailed")}: ${errorText(e)}`, false);
}

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** A reply in the outbox; shown as an outgoing bubble with a sending/failed receipt. */
export interface OutboxItem {
  id: string;
  chatId: string;
  body: string;
  /** ISO 8601; when the latest attempt started. */
  sentAt: string;
  delivery: Delivery;
  error?: string;
  /** Attached files; held in memory only, as they are too big for localStorage. */
  files?: File[];
  /** Id of the message this answers, when the user picked one. */
  replyTo?: string;
  /** The address it goes out from. */
  from?: string;
  /** Names of files a restart dropped from a failed reply (see `retrySend`). */
  lostFiles?: string[];
}

/** Failed replies survive a restart; one still sending when the app quit is not kept. */
function loadOutbox(): OutboxItem[] {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(OUTBOX_KEY) ?? "[]");
    return Array.isArray(saved)
      ? (saved as OutboxItem[]).filter((o) => o.delivery === "failed")
      : [];
  } catch {
    return [];
  }
}

function patchOutbox(id: string, fn: (item: OutboxItem) => OutboxItem) {
  useMail.setState((s) => ({ outbox: s.outbox.map((o) => (o.id === id ? fn(o) : o)) }));
}

async function deliver(item: OutboxItem) {
  try {
    await core.sendReply(
      item.chatId,
      item.body,
      await toUploads(item.files ?? []),
      item.replyTo,
      item.from,
    );
  } catch (e) {
    patchOutbox(item.id, (o) => ({ ...o, delivery: "failed", error: errorText(e) }));
    sendFailed(e);
    return;
  }
  // Swap the pending bubble for the stored message in one update, so neither flickers.
  const [list] = await Promise.all([
    core.chatMessages(item.chatId).catch(() => null),
    useMail.getState().loadChats(),
  ]);
  useMail.setState((s) => ({
    outbox: s.outbox.filter((o) => o.id !== item.id),
    messages: list ? { ...s.messages, [item.chatId]: list } : s.messages,
  }));
}

useMail.subscribe((s, prev) => {
  if (s.outbox === prev.outbox) return;
  try {
    const failed = s.outbox
      .filter((o) => o.delivery === "failed")
      .map(({ files, ...o }): OutboxItem =>
        files?.length ? { ...o, lostFiles: files.map((f) => f.name) } : o,
      );
    if (failed.length) localStorage.setItem(OUTBOX_KEY, JSON.stringify(failed));
    else localStorage.removeItem(OUTBOX_KEY);
  } catch {
    // Storage unavailable: failed replies just won't survive a restart.
  }
});

function setRead(id: string, read: boolean, unread = 1) {
  useMail.setState((s) => ({
    chats: updateChat(s.chats, id, (c) => ({
      ...c,
      unread: read ? 0 : Math.max(c.unread, unread),
    })),
  }));
  void core.setChatRead(id, read).catch(() => undefined);
}

/**
 * Archives on the server (Gmail label or Archive folder). The list updates at once; if the
 * server refuses, the chat goes back and a message explains why.
 */
function setArchived(id: string, archived: boolean) {
  useMail.setState((s) => ({ chats: updateChat(s.chats, id, (c) => ({ ...c, archived })) }));
  if (!IN_TAURI) return;
  const call = core.archiveChat(id, archived);
  pendingArchive.set(id, { archived, call });
  void call
    .catch((e: Error) => {
      useMail.setState((s) => ({
        chats: updateChat(s.chats, id, (c) => ({ ...c, archived: !archived })),
      }));
      showToast(`${i18n.t("toast.archiveFailed")}: ${e.message}`, false);
    })
    .finally(() => {
      if (pendingArchive.get(id)?.call === call) pendingArchive.delete(id);
    });
}

/**
 * Archive changes still on their way to the server. The database only has them once the
 * server confirms, so a reload in between (a sync finishing) would bring the chat back.
 */
const pendingArchive = new Map<string, { archived: boolean; call: Promise<unknown> }>();

function withPendingArchive(chats: Chat[]): Chat[] {
  if (!pendingArchive.size) return chats;
  return chats.map((c) => {
    const p = pendingArchive.get(c.id);
    return p ? { ...c, archived: p.archived } : c;
  });
}

function stateOf(c: Chat): ChatStateInput {
  return {
    pinned: c.pinned,
    muted: c.muted,
    archived: c.archived,
    manualStatus: c.manualStatus ?? null,
  };
}

/** Applies a pin/mute/archive/status change locally and saves it in the core. */
function patchChat(id: string, fn: (c: Chat) => Chat) {
  const chats = updateChat(useMail.getState().chats, id, fn);
  useMail.setState({ chats });
  const chat = chats.find((c) => c.id === id);
  if (chat) void core.setChatState(id, stateOf(chat)).catch(() => undefined);
}

/**
 * Chats the sidebar shows, in display order: pinned first, then most recent. While
 * searching, best match first.
 */
export function visibleChats(
  s: Pick<
    MailState,
    "chats" | "accountFilter" | "statusFilter" | "showArchived" | "query" | "search"
  >,
  showUnsubscribe = usePrefs.getState().showUnsubscribe,
): Chat[] {
  const byRank = searchOrder(s.query, s.search);
  return s.chats
    .filter((c) => c.archived === s.showArchived)
    .filter((c) => showUnsubscribe || !isUnsubscribeChat(c))
    .filter((c) => s.accountFilter === "all" || c.accountId === s.accountFilter)
    .filter((c) => s.statusFilter === "all" || effectiveStatus(c) === s.statusFilter)
    .filter((c) => matchesSearch(c, s.query, s.search))
    .sort((a, b) => {
      if (byRank) return byRank(a, b);
      if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
      return (lastMessage(b)?.sentAt ?? "").localeCompare(lastMessage(a)?.sentAt ?? "");
    });
}

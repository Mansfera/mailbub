import { create } from "zustand";
import i18n from "../i18n";
import { useMail } from "../store";
import { core } from "./core";
import { matchesSearch } from "./search";
import type { Chat, Mailbox, MailboxRole, Message } from "../types";

/**
 * Which mailbox the chat list shows. Inbox and Archive come from the synced cache; the rest
 * are read live from the server, a page at a time, and never cached.
 */
export type View =
  | { kind: "inbox" }
  | { kind: "archive" }
  /** The folder with this role in every account (Trash of each, side by side). */
  | { kind: "role"; role: BrowseRole }
  | { kind: "folder"; accountId: string; folder: string; label: string };

export const BROWSE_ROLES = ["sent", "drafts", "junk", "trash"] as const;
export type BrowseRole = (typeof BROWSE_ROLES)[number];

/** Menu order for "Move to"; other folders follow by name. */
const ROLE_ORDER: MailboxRole[] = ["inbox", "archive", "sent", "drafts", "junk", "trash"];

/** One server folder being browsed, and how far back it has been read. */
interface Source {
  accountId: string;
  folder: string;
  /** Cursor for the next page; null once everything is read. */
  before: number | null;
  started: boolean;
}

/** Where a browsed message lives on the server. */
export interface BrowsedRef {
  accountId: string;
  folder: string;
  uid: number;
}

interface MailboxState {
  view: View;
  /** Folders per account, loaded on first use. */
  boxes: Record<string, Mailbox[]>;
  sources: Source[];
  chats: Chat[];
  /** Messages of browsed chats, oldest first (like `useMail.messages`). */
  messages: Record<string, Message[]>;
  refs: Record<string, BrowsedRef>;
  loading: boolean;
  error: string | null;

  open: (view: View) => Promise<void>;
  more: () => Promise<void>;
  loadBoxes: (accountId: string) => Promise<Mailbox[]>;
  /** Moves messages of one account (stored or browsed) to another of its folders. */
  move: (accountId: string, messages: Message[], target: Mailbox) => Promise<void>;
}

const isRemote = (v: View) => v.kind === "role" || v.kind === "folder";

/** Label for a folder in menus: its role's name in the UI language, else its own. */
export function mailboxLabel(b: Mailbox): string {
  return b.role === "other" || b.role === "all" ? b.label : i18n.t(`mailbox.${b.role}`);
}

/** Folders to offer in "Move to", in a familiar order. */
export function moveTargets(boxes: Mailbox[]): Mailbox[] {
  const rank = (b: Mailbox) => {
    const i = ROLE_ORDER.indexOf(b.role);
    return i < 0 ? ROLE_ORDER.length : i;
  };
  return boxes
    .filter((b) => b.role !== "all")
    .sort((a, b) => rank(a) - rank(b) || a.label.localeCompare(b.label));
}

/** Groups browsed messages into chats, like the cache does: one per (account, others). */
function toChats(
  list: {
    accountId: string;
    folder: string;
    chatKey: string;
    others: Chat["participants"];
    message: Message;
  }[],
) {
  const byChat = new Map<string, { chat: Chat; messages: Message[] }>();
  for (const b of list) {
    const id = `box:${b.folder}:${b.chatKey}`;
    let entry = byChat.get(id);
    if (!entry) {
      entry = {
        chat: {
          id,
          accountId: b.accountId,
          participants: b.others,
          last: b.message,
          unread: 0,
          automated: false,
          pinned: false,
          muted: false,
          archived: false,
        },
        messages: [],
      };
      byChat.set(id, entry);
    }
    entry.messages.push(b.message);
  }
  const chats: Chat[] = [];
  const messages: Record<string, Message[]> = {};
  for (const [id, { chat, messages: list }] of byChat) {
    list.sort((a, b) => a.sentAt.localeCompare(b.sentAt));
    chats.push({
      ...chat,
      last: list[list.length - 1],
      unread: list.filter((m) => !m.read && !m.outgoing).length,
    });
    messages[id] = list;
  }
  return { chats, messages };
}

/** Everything browsed so far, kept flat so pages and moves can rebuild the chats. */
let loaded: {
  accountId: string;
  folder: string;
  chatKey: string;
  others: Chat["participants"];
  message: Message;
  uid: number;
}[] = [];
/** Bumped on every `open`, so pages of a mailbox the user left are dropped. */
let generation = 0;

function rebuild() {
  const { chats, messages } = toChats(loaded);
  const refs: Record<string, BrowsedRef> = {};
  for (const b of loaded)
    refs[b.message.id] = { accountId: b.accountId, folder: b.folder, uid: b.uid };
  useMailbox.setState({ chats, messages, refs });
}

export const useMailbox = create<MailboxState>()((set, get) => ({
  view: { kind: "inbox" },
  boxes: {},
  sources: [],
  chats: [],
  messages: {},
  refs: {},
  loading: false,
  error: null,

  open: async (view) => {
    generation++;
    loaded = [];
    const mail = useMail.getState();
    if (mail.selectedId?.startsWith("box:") || isRemote(get().view) !== isRemote(view)) {
      mail.select(null);
    }
    set({ view, sources: [], chats: [], messages: {}, refs: {}, error: null });
    if (view.kind === "inbox" || view.kind === "archive") {
      mail.setShowArchived(view.kind === "archive");
      return;
    }
    set({ loading: true });
    const gen = generation;
    try {
      let sources: Source[];
      if (view.kind === "folder") {
        sources = [
          { accountId: view.accountId, folder: view.folder, before: null, started: false },
        ];
      } else {
        const accounts = mail.accounts.map((a) => a.id);
        const boxes = await Promise.all(
          accounts.map((id) =>
            get()
              .loadBoxes(id)
              .catch(() => []),
          ),
        );
        sources = accounts.flatMap((accountId, i) => {
          const box = boxes[i].find((b) => b.role === view.role);
          return box ? [{ accountId, folder: box.name, before: null, started: false }] : [];
        });
      }
      if (gen !== generation) return;
      set({ sources });
      await get().more();
    } catch (e) {
      if (gen === generation) set({ error: (e as Error).message, loading: false });
    }
  },

  more: async () => {
    const gen = generation;
    const pending = get().sources.filter((s) => !s.started || s.before !== null);
    if (!pending.length) return set({ loading: false });
    set({ loading: true, error: null });
    const results = await Promise.allSettled(
      pending.map((s) => core.browseMailbox(s.accountId, s.folder, s.started ? s.before : null)),
    );
    if (gen !== generation) return;
    let error: string | null = null;
    const sources = get().sources.map((s) => {
      const i = pending.indexOf(s);
      if (i < 0) return s;
      const r = results[i];
      if (r.status === "rejected") {
        error = (r.reason as Error).message;
        return s;
      }
      loaded.push(
        ...r.value.messages.map((b) => ({
          accountId: s.accountId,
          folder: s.folder,
          chatKey: b.chatKey,
          others: b.others,
          message: b.message,
          uid: b.uid,
        })),
      );
      return { ...s, started: true, before: r.value.before };
    });
    set({ sources, loading: false, error });
    rebuild();
  },

  loadBoxes: async (accountId) => {
    const cached = get().boxes[accountId];
    if (cached) return cached;
    const boxes = await core.listMailboxes(accountId);
    set((s) => ({ boxes: { ...s.boxes, [accountId]: boxes } }));
    return boxes;
  },

  move: async (accountId, messages, target) => {
    const refs = get().refs;
    const browsed = messages.filter((m) => refs[m.id]);
    const stored = messages.filter((m) => !refs[m.id] && !m.delivery);
    const { notify } = useMail.getState();
    try {
      if (stored.length) {
        await core.moveStoredMessages(
          stored.map((m) => m.id),
          target.name,
        );
      }
      // Browsed mail moves per source folder.
      const byFolder = new Map<string, number[]>();
      for (const m of browsed) {
        const r = refs[m.id];
        byFolder.set(r.folder, [...(byFolder.get(r.folder) ?? []), r.uid]);
      }
      for (const [folder, uids] of byFolder) {
        await core.moveBrowsedMessages(accountId, folder, uids, target.name);
      }
      if (browsed.length) {
        const gone = new Set(browsed.map((m) => m.id));
        // Moved within the mailbox being shown (e.g. Trash → Trash) stays; else it left.
        loaded = loaded.filter((b) => !gone.has(b.message.id) || b.folder === target.name);
        rebuild();
        const selected = useMail.getState().selectedId;
        if (selected && !get().chats.some((c) => c.id === selected))
          useMail.getState().select(null);
      }
      notify(i18n.t("mailbox.moved", { count: messages.length, folder: mailboxLabel(target) }));
    } catch (e) {
      notify(`${i18n.t("mailbox.moveFailed")}: ${(e as Error).message}`);
    }
  },
}));

/** Browsed chats as the sidebar lists them: account filter and search applied, newest first. */
export function browsedChats(chats: Chat[], s: { accountFilter: string; query: string }): Chat[] {
  return chats
    .filter((c) => s.accountFilter === "all" || c.accountId === s.accountFilter)
    .filter((c) => matchesSearch(c, s.query, null))
    .sort((a, b) => b.last.sentAt.localeCompare(a.last.sentAt));
}

export const isBrowsedChat = (id: string | null) => Boolean(id?.startsWith("box:"));

/** Account of the open chat, local or browsed. */
export function selectedAccountId(): string | null {
  const id = useMail.getState().selectedId;
  if (!id) return null;
  const chat =
    useMail.getState().chats.find((c) => c.id === id) ??
    useMailbox.getState().chats.find((c) => c.id === id);
  return chat?.accountId ?? null;
}

export interface MoveItem {
  text: string;
  action?: () => void;
  enabled?: boolean;
}

/**
 * "Move to" items for messages of one account; the folder they are browsed in is left out.
 * `messages` may be fetched only when a folder is picked (a chat that was never opened).
 * If the folders can't be listed, one disabled item says why, so the menu entry never
 * just goes missing.
 */
export async function moveMenuItems(
  accountId: string,
  messages: Message[] | (() => Promise<Message[]>),
): Promise<MoveItem[]> {
  const { loadBoxes, refs } = useMailbox.getState();
  let boxes: Mailbox[];
  try {
    boxes = await loadBoxes(accountId);
  } catch (e) {
    return [
      { text: i18n.t("mailbox.foldersFailed", { error: (e as Error).message }), enabled: false },
    ];
  }
  const known = Array.isArray(messages) ? messages : [];
  const from = new Set(known.map((m) => refs[m.id]?.folder));
  const here = from.size === 1 ? [...from][0] : undefined;
  const resolve = Array.isArray(messages) ? async () => messages : messages;
  return moveTargets(boxes)
    .filter((b) => b.name !== here)
    .map((b) => ({
      text: mailboxLabel(b),
      action: () =>
        void resolve()
          .then((list) => useMailbox.getState().move(accountId, list, b))
          .catch((e: Error) =>
            useMail.getState().notify(`${i18n.t("mailbox.moveFailed")}: ${e.message}`),
          ),
    }));
}

import { invoke } from "@tauri-apps/api/core";
import type { FileUpload } from "./draftFiles";
import type {
  Backfill,
  BrowsedMessage,
  Chat,
  ChatStatus,
  Message,
  RemoteRule,
  SearchHit,
  StorageUsage,
  SyncStatus,
  Account,
  AccountDraft,
  AccountUpdate,
  BrowserStart,
  Discovered,
  ImportCandidate,
  Mailbox,
  SecretInput,
} from "../types";

export const IN_TAURI = "__TAURI_INTERNALS__" in window;
export const IS_ANDROID = /Android/i.test(navigator.userAgent);

/** Mirrors `error::ErrorCode` in the Rust core. */
export type ErrorCode =
  | "invalid"
  | "duplicate"
  | "not_found"
  | "storage"
  | "keychain"
  | "timeout"
  | "imap_connect"
  | "imap_tls"
  | "imap_auth"
  | "smtp_connect"
  | "smtp_auth"
  | "oauth_not_configured"
  | "oauth_pending"
  | "mail_not_allowed"
  | "mail_unavailable"
  | "oauth_denied"
  | "oauth_expired"
  | "oauth"
  | "no_core";

export class CoreError extends Error {
  constructor(
    public code: ErrorCode,
    message: string,
  ) {
    super(message);
  }
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!IN_TAURI) throw new CoreError("no_core", "Open the desktop app to manage accounts.");
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    const err = e as { code?: ErrorCode; message?: string } | string;
    if (typeof err === "object" && err.code) throw new CoreError(err.code, err.message ?? "");
    throw new CoreError("storage", String(e));
  }
}

/** The core sends Unix seconds; the UI formats ISO strings. */
type Wire<T> = Omit<T, "sentAt"> & { sentAt: number };
const toMessage = (m: Wire<Message>): Message => ({
  ...m,
  sentAt: new Date(m.sentAt * 1000).toISOString(),
  quoted: m.quoted ?? undefined,
});

export interface NewMessage {
  accountId: string;
  /** One or more addresses, separated by commas. */
  to: string;
  subject: string;
  body: string;
  attachments?: FileUpload[];
  /** An address of the account; omitted means its default. */
  from?: string;
}

export interface ChatStateInput {
  pinned: boolean;
  muted: boolean;
  archived: boolean;
  manualStatus: ChatStatus | null;
}

export const core = {
  listChats: async () =>
    (await call<(Omit<Chat, "last"> & { last: Wire<Message> })[]>("list_chats")).map((c) => ({
      ...c,
      last: toMessage(c.last),
    })),
  chatMessages: async (chatId: string) =>
    (await call<Wire<Message>[]>("chat_messages", { chatId })).map(toMessage),
  searchChats: (query: string, limit?: number) =>
    call<SearchHit[]>("search_chats", { query, limit }),
  loadOlderMail: (accountIds: string[]) => call<Backfill[]>("load_older_mail", { accountIds }),
  setChatRead: (chatId: string, read: boolean) => call<null>("set_chat_read", { chatId, read }),
  setChatState: (chatId: string, chatState: ChatStateInput) =>
    call<null>("set_chat_state", { chatId, chatState }),
  syncStatus: () => call<SyncStatus>("sync_status"),
  syncNow: () => call<null>("sync_now"),
  storageUsage: () => call<StorageUsage>("storage_usage"),
  /** Clears cached mail and files, then downloads the last 30 days again. Resolves when done. */
  resetCache: () => call<StorageUsage>("reset_cache"),
  deleteChat: (chatId: string) => call<null>("delete_chat", { chatId }),
  archiveChat: (chatId: string, archived: boolean) =>
    call<null>("archive_chat", { chatId, archived }),
  /** Replies to `replyTo` (a message in the chat), or else to the newest message. */
  sendReply: (
    chatId: string,
    body: string,
    attachments: FileUpload[] = [],
    replyTo?: string,
    from?: string,
  ) =>
    call<null>("send_reply", {
      chatId,
      body,
      attachments,
      replyTo: replyTo ?? null,
      from: from ?? null,
    }),
  /** Returns the id of the chat the message landed in. */
  sendMessage: (draft: NewMessage) => call<string>("send_message", { ...draft }),
  listRemoteRules: () => call<RemoteRule[]>("list_remote_rules"),
  addToCalendar: (event: {
    title: string;
    start: number;
    end: number | null;
    allDay: boolean;
    location: string | null;
    notes: string | null;
  }) => call<null>("add_to_calendar", { event }),
  openExternal: (url: string) => call<null>("open_external", { url }),
  setSystemBars: (dark: boolean) => call<null>("set_system_bars", { dark }),
  setSignInActive: (active: boolean) => call<null>("set_sign_in_active", { active }),
  setMailFrameHover: (over: boolean) => call<null>("set_mail_frame_hover", { over }),
  unsubscribeOneClick: (url: string) => call<null>("unsubscribe_one_click", { url }),
  listMailboxes: (accountId: string) => call<Mailbox[]>("list_mailboxes", { accountId }),
  /** Newest first; `before` is the cursor for the next (older) page, null at the end. */
  browseMailbox: async (accountId: string, folder: string, before: number | null) => {
    const page = await call<{
      messages: (Omit<BrowsedMessage, "message"> & { message: Wire<Message> })[];
      before: number | null;
    }>("browse_mailbox", { accountId, folder, before });
    return {
      before: page.before,
      messages: page.messages.map((b) => ({ ...b, message: toMessage(b.message) })),
    };
  },
  moveStoredMessages: (messageIds: string[], target: string) =>
    call<null>("move_stored_messages", { messageIds, target }),
  moveBrowsedMessages: (accountId: string, folder: string, uids: number[], target: string) =>
    call<null>("move_browsed_messages", { accountId, folder, uids, target }),
  openAttachment: (messageId: string, attachmentId: string) =>
    call<null>("open_attachment", { messageId, attachmentId }),
  geocode: (query: string) =>
    call<{ lat: number; lon: number; name: string } | null>("geocode", { query }),
  addRemoteRules: (rules: RemoteRule[]) => call<null>("add_remote_rules", { rules }),
  removeRemoteRule: (rule: RemoteRule) => call<null>("remove_remote_rule", { rule }),
  restoreChat: (chatId: string) => call<null>("restore_chat", { chatId }),
  listAccounts: () => call<Account[]>("list_accounts"),
  discoverServers: (email: string) => call<Discovered>("discover_servers", { email }),
  addAccount: (draft: AccountDraft, secret: SecretInput) =>
    call<{ account: Account; smtpWarning: string | null }>("add_account", { draft, secret }),
  updateAccount: (id: string, update: AccountUpdate) =>
    call<{ account: Account; smtpWarning: string | null }>("update_account", { id, update }),
  removeAccount: (id: string) => call<null>("remove_account", { id }),
  msOauthAvailable: () => (IN_TAURI ? call<boolean>("ms_oauth_available") : Promise.resolve(false)),
  msOauthStart: (email: string) => call<BrowserStart>("ms_oauth_start", { email }),
  listMailAppAccounts: () => call<ImportCandidate[]>("list_mail_app_accounts"),
  googleOauthAvailable: () =>
    IN_TAURI ? call<boolean>("google_oauth_available") : Promise.resolve(false),
  googleOauthStart: (email: string) => call<BrowserStart>("google_oauth_start", { email }),
  oauthPoll: (session: string) => call<null>("oauth_poll", { session }),
};

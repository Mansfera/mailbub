export type Provider = "gmail" | "microsoft" | "imap";
export type Security = "tls" | "starttls";

export const ACCOUNT_COLORS = ["red", "teal", "gold", "plum", "olive", "slate"] as const;
export type AccountColor = (typeof ACCOUNT_COLORS)[number];

export interface Server {
  host: string;
  port: number;
  security: Security;
}

/** Mirrors `model::Account` in the Rust core. */
export interface Account {
  id: string;
  provider: Provider;
  label: string;
  email: string;
  /** Login name for IMAP/SMTP; usually the address. */
  username: string;
  displayName: string;
  /** Other addresses of this mailbox: it receives at them and can send from them. */
  aliases: string[];
  /** The address new messages go out from: `email` or one of `aliases`. */
  defaultFrom: string;
  color: AccountColor;
  imap: Server;
  smtp: Server;
  auth: "password" | "oauth";
  createdAt: number;
}

export type AccountDraft = Omit<Account, "id" | "auth" | "createdAt" | "defaultFrom">;

/** Mirrors `model::AccountUpdate`: the fields account settings can change. */
export type AccountUpdate = Pick<
  Account,
  "label" | "displayName" | "aliases" | "defaultFrom" | "color" | "username" | "imap" | "smtp"
> & {
  /** New password; omitted keeps the stored one. */
  password?: string;
};

export type SecretInput =
  { kind: "password"; password: string } | { kind: "oauth"; session: string };

export interface Discovered {
  provider: Provider;
  imap: Server | null;
  smtp: Server | null;
}

/** Mirrors `model::ImportCandidate`: an account found in Apple Mail. */
export interface ImportCandidate {
  name: string;
  email: string;
  aliases: string[];
  username: string;
  displayName: string;
  provider: Provider;
  imap: Server | null;
  smtp: Server | null;
  alreadyConnected: boolean;
}

export interface BrowserStart {
  session: string;
  url: string;
}

export interface Person {
  name: string;
  address: string;
}

export type ChatStatus = "needs_reply" | "waiting" | "done";

export interface Attachment {
  id: string;
  name: string;
  size: number;
}

export interface Message {
  id: string;
  from: Person;
  outgoing: boolean;
  /** Which of the account's addresses an incoming message was sent to, when known. */
  ownAddress?: string | null;
  /** ISO 8601 timestamp. */
  sentAt: string;
  subject: string;
  /** New text only: quoted replies and signatures already stripped. */
  body: string;
  /** Stripped part, shown on "Show full message". */
  quoted?: string;
  /** Raw HTML body; present once a chat is opened. Sanitised before display. */
  html?: string | null;
  attachments: Attachment[];
  read: boolean;
  /** Mirrors `unsubscribe::Unsubscribe`: the List-Unsubscribe header, once a chat is opened. */
  unsubscribe?: { url: string | null; mailto: string | null; oneClick: boolean } | null;
  /** Only on replies still in the outbox; an outgoing message without it was sent. */
  delivery?: Delivery;
  /** Why sending failed (with `delivery: "failed"`). */
  deliveryError?: string;
  /** Id of the message in this chat that this one replies to; set once a chat is opened. */
  replyTo?: string | null;
}

export type Delivery = "sending" | "failed";

export interface Chat {
  id: string;
  accountId: string;
  /** Other participants (never includes the account owner). */
  participants: Person[];
  /** Newest message; the full list is loaded when the chat opens. */
  last: Message;
  unread: number;
  /** Only machines wrote and you never replied, or it is with no-reply addresses: Done. */
  automated: boolean;
  pinned: boolean;
  muted: boolean;
  archived: boolean;
  /** Set when the user picks a status by hand; otherwise it is derived. */
  manualStatus?: ChatStatus | null;
}

/** Mirrors `mailbox::MailboxRole`. */
export type MailboxRole =
  "inbox" | "sent" | "drafts" | "archive" | "junk" | "trash" | "all" | "other";

/** Mirrors `mailbox::Mailbox`: one IMAP folder of an account. */
export interface Mailbox {
  /** IMAP name, passed back to the core as is. */
  name: string;
  /** Human name. */
  label: string;
  role: MailboxRole;
  /** Synced into the cache: its mail already shows in chats. */
  synced: boolean;
}

/** Mirrors `mailbox::Browsed` (message already converted for the UI). */
export interface BrowsedMessage {
  uid: number;
  chatKey: string;
  others: Person[];
  message: Message;
}

/** "Always load remote content" rule; mirrors `db::RemoteRule`. */
export interface RemoteRule {
  kind: "host" | "sender" | "domain";
  value: string;
}

/** Mirrors `db::SearchHit`: a chat matching a full-text search. */
export interface SearchHit {
  chatId: string;
  messageId: string;
  /** Excerpt of the best-matching message; matches are wrapped in \u0002 … \u0003. */
  snippet: string | null;
}

/** Mirrors `sync::Backfill`: how fetching older mail went for one account. */
export interface Backfill {
  accountId: string;
  added: number;
  /** Older mail is still left on the server. */
  more: boolean;
  error: string | null;
}

/** Mirrors `storage::StorageUsage`: bytes on disk, and how many messages are cached. */
export interface StorageUsage {
  /** Mail and the search index. */
  database: number;
  attachments: number;
  avatars: number;
  messages: number;
}

/** Mirrors `model::SyncStatus`. */
export interface SyncStatus {
  running: boolean;
  /** Unix seconds. */
  lastRun: number | null;
  errors: { accountId: string; message: string }[];
}

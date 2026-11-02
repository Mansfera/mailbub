# Project: Mail-as-Chat (name: "Mailbub")

Desktop app (macOS first, Android later) that pulls email from several accounts and presents it as a messenger: a list of chats on the left, message bubbles on the right, reply box at the bottom. Goal: easy tracking of correspondence.

UI strings in English + Ukrainian, i18n from day one.

## Accounts to support
1. **Gmail**: Google sign-in or an app password.
2. **Custom domains and other providers**: generic IMAP 993 + SMTP 465/587 with password auth; servers autodetected from MX and common host names.
3. **Microsoft 365 / Outlook**: OAuth2 in the browser. Organisation admins may block IMAP or require approval for third-party apps; surface that clearly and consider Microsoft Graph as a fallback.

## Stack (default decision, change only with a reason)
- **Tauri 2** (Rust core + web UI). Same codebase targets macOS now and Android later (Tauri 2 mobile).
- UI: React + TypeScript + Vite, Tailwind, Zustand or TanStack Query.
- Core (Rust): `async-imap` (IMAP + XOAUTH2), `lettre` (SMTP), `mail-parser`, `rusqlite` or `sqlx` (SQLite, FTS5 for search), `oauth2` crate.
- Secrets: macOS Keychain (`keyring` crate); Android Keystore later. Never store passwords or tokens in plaintext or in the repo.
- Alternative if native feel matters more than Android reuse: SwiftUI app + shared nothing. Do not switch without asking.

## Architecture
- Local-first: all mail synced into local SQLite; UI reads only from the DB via Tauri commands/events.
- Per-account sync worker: initial sync (recent N days, configurable), then IMAP IDLE for push, with polling fallback. Track UIDVALIDITY/UID per folder (INBOX, Sent, optionally All Mail/Archive).
- Auth per account type:
  - Gmail: app password over IMAP for MVP; OAuth2 (XOAUTH2) as a later upgrade.
  - Microsoft (Exchange/Outlook): OAuth2 XOAUTH2 against IMAP/SMTP, or Graph API. Basic auth is disabled on Exchange Online.
- Sending: SMTP from the account that owns the chat; reply sets `In-Reply-To`/`References`; saves to Sent.

## Core concept: chats
- A **chat** = conversation with a counterpart, keyed by (account, normalized set of other participants). One-to-one by default; multi-recipient threads become group chats.
- Inside a chat, messages are in chronological order as bubbles (incoming left, outgoing right). Subject changes shown as small divider chips.
- Bubble shows the *new* text only: strip quoted replies and signatures (heuristics: `>` quotes, "On ... wrote:", Outlook headers, `-- ` signature). "Show full message" expands the original.
- HTML emails rendered sanitized in a sandboxed iframe, remote images blocked by default (tracking pixels).
- Attachments shown as file chips; download on click.
- Unified inbox across all accounts, with per-account color tag; filter by account.

## Tracking features (what makes this more than a viewer)
- Unread counts, pin, mute, archive.
- Chat status: *Needs reply* / *Waiting for them* / *Done* (auto-suggest "Waiting" after I send last message; "Needs reply" when they send last).
- Snooze / reminder ("remind me if no reply in 3 days").
- Labels, full-text search (SQLite FTS5).
- Keyboard-first: Cmd+K quick switcher, Cmd+N new message, j/k navigation, Cmd+Enter send.

## MVP milestones
1. Scaffold Tauri 2 + React app; chat-style UI on mock data; light/dark theme, macOS-native feel (vibrancy, traffic-light inset).
2. SQLite schema + one account (Gmail via IMAP app password): sync, threading into chats, render.
3. Send/reply via SMTP; mark read/unread sync back to server.
4. Add custom-domain IMAP and Microsoft 365 accounts; resolve OAuth/Graph as needed.
5. Tracking features (status, snooze, labels, search).
6. Package: signed/notarized `.dmg` (document steps; Apple Developer account may be needed).
7. Android build with responsive layout (list ↔ conversation navigation). `bun run android:apk` writes a signed arm64 APK to `dist/release/` (any later `vite build` empties `dist/`). Signing key: `~/.android/keystores/com.mansfera.mailbub.jks`, password in the macOS Keychain (`com.mansfera.mailbub.android-keystore`); an installed app only accepts updates signed with that key.

## Conventions
- Rust: `cargo fmt`, `clippy` clean. TS: strict mode, ESLint + Prettier.
- Small commits, conventional commit messages. Tests for threading/quote-stripping logic with real-world sample emails (anonymized fixtures).
- Do not send real emails or touch real mailboxes in tests; use a local IMAP test server or fixtures.

## OAuth configuration
Client ids live in an untracked `.env.local` (`MAILBUB_GOOGLE_CLIENT_ID`, `MAILBUB_GOOGLE_CLIENT_SECRET`, `MAILBUB_MS_CLIENT_ID`). The Microsoft Entra app registration is multi-tenant + personal accounts, with the mobile/desktop redirect URI `http://localhost`.

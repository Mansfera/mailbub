<img src="src-tauri/icons/128x128@2x.png" width="128" alt="Mailbub icon: an envelope shaped like a chat bubble">

# Mailbub

A macOS app that shows email from all your accounts as a messenger: conversations on the left, message bubbles on the right, a reply box at the bottom.

Requires macOS on Apple silicon. An arm64 Android build can be made from source.

## Install

```sh
brew install --cask mansfera/tap/mailbub
```

If Homebrew asks you to trust the tap first, run `brew trust --tap mansfera/tap`. To update, run `brew upgrade --cask mailbub`.

Or download the DMG from [Releases](https://github.com/Mansfera/mailbub/releases) and drag the app to Applications.

The app is not notarized. The Homebrew cask clears the quarantine flag after install. With the DMG, the first launch needs right-click → Open, or System Settings → Privacy & Security → Open Anyway.

To build it yourself:

```sh
bun install
bun run release
open dist/release
```

## Features

### Conversations
One chat per person or group, per account, in a unified inbox with a colour tag for each account and a filter by account. Bubbles show only the new text: quoted replies and signatures are stripped, and **Show full message** brings them back. A subject change appears as a divider in the chat.

### Tracking
- A status on every chat: Needs reply, Waiting or Done, suggested from the last message until you set one yourself.
- Pin, mute, archive, mark as unread.
- Full-text search across synced mail, and a quick switcher on ⌘K.
- Browse Inbox, Archive, Sent, Drafts, Junk, Trash and your own folders, move messages between them, and load older mail on demand.

### Writing
Reply or start a new message from any account, with attachments up to 25 MB in total. Aliases are supported: new messages use the address you pick, replies use the one the mail was sent to. A failed send stays in the chat, ready to retry or edit.

### Reading
- **Readable mode** shows HTML mail in the app's colours and font. Right-click a message for the original.
- Images from the web are blocked until you allow them by sender, domain or image source.
- One-click unsubscribe for mailing lists.
- No-reply senders can show as read-only chats at full width.
- Add events to Calendar or Google Calendar, join meetings, answer invitations, and open places in Apple Maps or Google Maps.

### Privacy and storage
Mail is kept on your device and read from there, so the app works offline and search is instant. Passwords and sign-in tokens go to the system keychain. Settings → Storage shows what's used and can clear the cache, which deletes nothing on the server. Profile pictures from Gravatar and company logos can be turned off.

### Appearance and language
Light, dark or system theme. English and Ukrainian.

## Accounts

| Connector | Sign-in | Notes |
|---|---|---|
| Gmail | Sign in with Google, or an app password | App passwords need 2-Step Verification |
| Microsoft 365 / Outlook | Sign in with Microsoft in the browser | Organisations may require admin approval |
| Other mail (IMAP) | Password over IMAP and SMTP | Servers are detected from MX records and common host names |
| Import from Apple Mail | Password for each account | Reads account settings from Mail after one Automation prompt. macOS only |

Adding an account logs in to IMAP (required) and SMTP (a warning if it fails).

## Keyboard

| Key | Action |
|---|---|
| ⌘K | Jump to a conversation |
| ⌘N | New message |
| ⌘R | Check for mail |
| j / k | Next / previous conversation |
| r or Enter | Focus the reply box |
| ⌘↵ | Send |
| e | Archive / unarchive |
| / | Search the list |
| Esc | Close / deselect |

## Where things live

| What | Where |
|---|---|
| Bundle ID | `com.mansfera.mailbub` |
| Mail, search index and accounts | `~/Library/Application Support/com.mansfera.mailbub/mailbub.db` |
| Passwords and tokens | macOS Keychain, service `com.mansfera.mailbub` |

`brew uninstall --zap mailbub` also removes local mail and settings. Remove accounts in Settings first to delete their Keychain entries.

## Not supported

- Snooze, reminders and labels, which aren't built yet.
- Microsoft organisations that block IMAP or third-party apps, unless an admin approves the app.
- Intel Macs.

## Development

Tauri 2 with a Rust core and a React + TypeScript UI. Needs [Bun](https://bun.sh), Rust via rustup (with `rustfmt` and `clippy`) and the Xcode command-line tools.

```sh
bun install
bun tauri dev          # native window
bun run dev            # browser only, http://localhost:1420
```

```sh
bun run check          # tsc + eslint + prettier
bun run test           # UI logic tests
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cargo test -- --ignored   # live DNS and server checks for autodetect
```

Project brief and roadmap are in [CLAUDE.md](CLAUDE.md). The UI is in `src/` and the core (sync, SQLite, IMAP, SMTP, sign-in) is in `src-tauri/src/`.

### OAuth client ids

Google and Microsoft sign-in need your own client ids in an untracked `.env.local`, which `src-tauri/build.rs` compiles in:

```sh
MAILBUB_GOOGLE_CLIENT_ID=...
MAILBUB_GOOGLE_CLIENT_SECRET=...
MAILBUB_MS_CLIENT_ID=...
```

Google needs a Google Cloud OAuth client of type Desktop app. Microsoft needs an Entra app registration with the mobile/desktop redirect URI `http://localhost`. Without them, app passwords and IMAP still work.

### Code signing

Builds are signed with a free local identity named **Mailbub Code Signing**, so macOS recognises each rebuild as the same app and Keychain "Always Allow" sticks.

1. In Keychain Access, open **Keychain Access → Certificate Assistant → Create a Certificate…**
2. Name it `Mailbub Code Signing`, with Identity Type **Self-Signed Root** and Certificate Type **Code Signing**.
3. Double-click the new certificate and set **Trust → Code Signing** to **Always Trust**.

Without the identity, dev builds run unsigned, `bun run release` signs ad-hoc, and `bun tauri build` fails.

### Android

`bun run android:apk` writes a signed arm64 APK to `dist/release/`. The keystore is created in `~/.android/keystores/` on the first run, with its password in the macOS Keychain. Back them up: an installed app only accepts updates signed with the same key.

### Releasing

Bump `version` in `src-tauri/tauri.conf.json`, then:

```sh
bun run release               # builds the DMG and cask into dist/release/
bun run release -- --publish  # also creates the GitHub release and updates the tap
```

`--publish` uses `RELEASE_REPO` (default `Mansfera/mailbub`, which must be public for Homebrew to download from it) and `TAP_REPO` (default [Mansfera/homebrew-tap](https://github.com/Mansfera/homebrew-tap)).

## Credits

Developed by [mansfera](https://github.com/Mansfera). Made with Claude.

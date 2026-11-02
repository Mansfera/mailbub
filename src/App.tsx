import { useEffect } from "react";
import { exit, onBackButtonPress } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { core, IS_ANDROID } from "./lib/core";
import type { SyncStatus } from "./types";
import { useMail, visibleChats } from "./store";
import { browsedChats, useMailbox } from "./lib/mailbox";
import { useLanguage, usePrefsFromOtherWindows, useTheme } from "./lib/appearance";
import { useMenuSheet } from "./lib/popupMenu";
import { openSettingsWindow, useSettingsPage } from "./lib/settingsWindow";
import { Sidebar } from "./components/Sidebar";
import { Conversation } from "./components/Conversation";
import { QuickSwitcher } from "./components/QuickSwitcher";
import { ComposeDialog } from "./components/ComposeDialog";
import { AccountSettingsDialog } from "./components/AccountSettingsDialog";
import { AddAccountDialog } from "./components/AddAccountDialog";
import { MenuSheet } from "./components/MenuSheet";
import { SettingsWindow } from "./components/SettingsWindow";
import { Toast } from "./components/Toast";

const IN_TAURI = "__TAURI_INTERNALS__" in window;

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  return Boolean(el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)));
}

function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mail = useMail.getState();
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (mail.setup) return;
        mail.setSwitcherOpen(!mail.switcherOpen);
        return;
      }
      // ⌘Z reverts the last chat action; inside text fields it stays the native text undo.
      if (mod && !e.shiftKey && e.key.toLowerCase() === "z" && !isTyping(e.target)) {
        e.preventDefault();
        if (!mail.setup && !mail.composeOpen) mail.undo();
        return;
      }
      if (mod && e.key === ",") {
        e.preventDefault();
        void openSettingsWindow();
        return;
      }
      // ⌘R checks mail instead of reloading the web view.
      if (mod && e.key.toLowerCase() === "r") {
        e.preventDefault();
        if (mail.accounts.length) mail.syncNow();
        return;
      }
      if (mod && e.key.toLowerCase() === "n") {
        e.preventDefault();
        if (mail.setup) return;
        if (mail.accounts.length) mail.setComposeOpen(true);
        else mail.setSetup("choose");
        return;
      }
      if (
        mail.setup ||
        mail.switcherOpen ||
        mail.composeOpen ||
        mod ||
        e.altKey ||
        isTyping(e.target)
      )
        return;

      if (e.key === "j" || e.key === "k") {
        const box = useMailbox.getState();
        const remote = box.view.kind === "role" || box.view.kind === "folder";
        const list = remote ? browsedChats(box.chats, mail) : visibleChats(mail);
        if (list.length === 0) return;
        const idx = list.findIndex((c) => c.id === mail.selectedId);
        const next =
          idx < 0 ? 0 : Math.min(Math.max(idx + (e.key === "j" ? 1 : -1), 0), list.length - 1);
        if (remote) useMail.setState({ selectedId: list[next].id, unreadFrom: null });
        else mail.select(list[next].id);
        document
          .querySelector(`[data-chat-id="${list[next].id}"]`)
          ?.scrollIntoView({ block: "nearest" });
      } else if (e.key === "/") {
        e.preventDefault();
        document.getElementById("sidebar-search")?.focus();
      } else if ((e.key === "r" || e.key === "Enter") && mail.selectedId) {
        e.preventDefault();
        document.getElementById("composer")?.focus();
      } else if ((e.key === "Backspace" || e.key === "Delete") && mail.selectedId) {
        e.preventDefault();
        mail.deleteChat(mail.selectedId);
      } else if (e.key === "e" && mail.selectedId) {
        mail.toggleArchive(mail.selectedId);
      } else if (e.key === "Escape" && mail.selectedId) {
        mail.select(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

/** Android's back: closes what's open the way Esc does, and leaves from the chat list. */
function useBackButton() {
  useEffect(() => {
    if (!IS_ANDROID) return;
    const listener = onBackButtonPress(() => {
      const open =
        useMail.getState().selectedId ||
        useSettingsPage.getState().open ||
        document.querySelector('[role="dialog"]');
      if (!open) return void exit(0);
      document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    return () => void listener.then((l) => l.unregister());
  }, []);
}

const closeSettingsPage = () => useSettingsPage.setState({ open: false });

export default function App() {
  useBackButton();
  useTheme();
  useLanguage();
  usePrefsFromOtherWindows();
  useShortcuts();
  const switcherOpen = useMail((s) => s.switcherOpen);
  const composeOpen = useMail((s) => s.composeOpen);
  const setup = useMail((s) => s.setup);
  const editingAccount = useMail((s) => s.editingAccount);
  const settingsPage = useSettingsPage((s) => s.open);
  const menuSheet = useMenuSheet((s) => s.items);

  useEffect(() => {
    const mail = useMail.getState();
    void mail.loadAccounts();
    void mail.loadChats();
    void mail.loadRemoteRules();
    if (!IN_TAURI) return;
    void core
      .syncStatus()
      .then(mail.setSync)
      .catch(() => undefined);
    const unlisten = [
      listen("mail-changed", () => {
        const m = useMail.getState();
        void m.loadChats();
        if (m.selectedId) void m.loadMessages(m.selectedId);
      }),
      // Changes made in the Settings window.
      listen("accounts-changed", () => {
        const m = useMail.getState();
        void m.loadAccounts();
        void m.loadChats();
      }),
      listen("rules-changed", () => void useMail.getState().loadRemoteRules()),
      listen<SyncStatus>("sync-status", (e) => {
        useMail.getState().setSync(e.payload);
        if (!e.payload.running) void useMail.getState().loadChats();
      }),
    ];
    return () => unlisten.forEach((p) => void p.then((off) => off()));
  }, []);

  return (
    <div className="flex h-full">
      <Sidebar />
      <Conversation />
      {settingsPage && (
        <div className="fixed inset-0 z-30">
          <SettingsWindow onClose={closeSettingsPage} />
        </div>
      )}
      {switcherOpen && <QuickSwitcher />}
      {composeOpen && <ComposeDialog />}
      {setup && <AddAccountDialog />}
      <Toast />
      {editingAccount && <AccountSettingsDialog />}
      {menuSheet && <MenuSheet items={menuSheet} />}
    </div>
  );
}

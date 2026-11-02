import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AtSign,
  ChevronLeft,
  HardDrive,
  Loader2,
  Mail,
  Pencil,
  Plus,
  Shield,
  SlidersHorizontal,
  Trash2,
  X,
} from "lucide-react";
import { LANGUAGES } from "../i18n";
import { useMail, usePrefs, type Theme } from "../store";
import { core, IN_TAURI, IS_ANDROID } from "../lib/core";
import { formatSize } from "../lib/format";
import { useLanguage, usePrefsFromOtherWindows, useTheme } from "../lib/appearance";
import { initialSettingsTab, type SettingsTab } from "../lib/settingsWindow";
import type { Account, StorageUsage } from "../types";
import { AccountDot, IconButton } from "./primitives";
import { AddAccountDialog } from "./AddAccountDialog";
import { AccountSettingsDialog } from "./AccountSettingsDialog";
import { Toast } from "./Toast";

const TABS: { id: SettingsTab; icon: typeof Mail }[] = [
  { id: "general", icon: SlidersHorizontal },
  { id: "accounts", icon: AtSign },
  { id: "mail", icon: Mail },
  { id: "privacy", icon: Shield },
  { id: "storage", icon: HardDrive },
];

/** Mirrors `storage::RESYNC_DAYS`. */
const RESYNC_DAYS = 30;

const THEMES: Theme[] = ["system", "light", "dark"];

function Segmented<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex rounded-md bg-hover p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          onClick={() => onChange(o.value)}
          className={`h-6 rounded px-2.5 text-[12px] whitespace-nowrap ${
            value === o.value
              ? "bg-raised font-medium text-text shadow-sm"
              : "text-text-2 hover:text-text"
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function Switch({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={`relative h-[22px] w-[38px] shrink-0 rounded-full transition-colors ${
        checked ? "bg-brand" : "bg-text-3/40"
      }`}
    >
      <span
        aria-hidden
        className={`absolute top-[2px] size-[18px] rounded-full bg-white shadow transition-[left] ${
          checked ? "left-[18px]" : "left-[2px]"
        }`}
      />
    </button>
  );
}

/** A grouped list like macOS System Settings: a heading and rows in a rounded box. */
function Group({
  title,
  footer,
  children,
}: {
  title?: string;
  footer?: string;
  children: ReactNode;
}) {
  return (
    <section className="mb-5">
      {title && <h2 className="mb-1.5 px-1 text-[12px] font-medium text-text-2">{title}</h2>}
      <div className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-raised">
        {children}
      </div>
      {footer && <p className="mt-1.5 px-1 text-[11px] leading-relaxed text-text-2">{footer}</p>}
    </section>
  );
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex min-h-11 items-center justify-between gap-4 px-3.5 py-2">
      <div className="min-w-0">
        <div className="text-[13px]">{label}</div>
        {hint && <div className="text-[11px] text-text-2">{hint}</div>}
      </div>
      {children}
    </div>
  );
}

function GeneralTab() {
  const { t } = useTranslation();
  const { theme, language, setTheme, setLanguage } = usePrefs();
  return (
    <Group>
      <Row label={t("settings.theme")}>
        <Segmented
          label={t("settings.theme")}
          value={theme}
          onChange={setTheme}
          options={THEMES.map((v) => ({ value: v, label: t(`settings.${v}`) }))}
        />
      </Row>
      <Row label={t("settings.language")}>
        <Segmented
          label={t("settings.language")}
          value={language}
          onChange={setLanguage}
          options={LANGUAGES.map((l) => ({ value: l.code, label: l.label }))}
        />
      </Row>
    </Group>
  );
}

function AccountsTab() {
  const { t } = useTranslation();
  const accounts = useMail((s) => s.accounts);
  const setSetup = useMail((s) => s.setSetup);
  const setEditingAccount = useMail((s) => s.setEditingAccount);
  const removeAccount = useMail((s) => s.removeAccount);
  const [confirming, setConfirming] = useState<Account | null>(null);
  const [error, setError] = useState<string | null>(null);

  return (
    <>
      <Group footer={t("settingsWindow.accountsFooter")}>
        {accounts.length === 0 && (
          <p className="px-3.5 py-3 text-[12px] text-text-2">{t("settingsWindow.noAccounts")}</p>
        )}
        {accounts.map((a) => (
          <div key={a.id} className="group flex items-center gap-3 px-3.5 py-2.5">
            <AccountDot color={a.color} className="size-2.5" />
            <div className="min-w-0 flex-1">
              <div className="truncate text-[13px] font-medium">{a.label}</div>
              <div className="truncate text-[11px] text-text-2">
                {a.email} · {t(`setup.provider.${a.provider}`)}
              </div>
            </div>
            <button
              type="button"
              onClick={() => setEditingAccount(a.id)}
              className="flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[12px] text-text-2 hover:bg-hover hover:text-text"
            >
              <Pencil size={13} aria-hidden />
              {t("settingsWindow.edit")}
            </button>
            <button
              type="button"
              aria-label={`${t("accounts.remove")} ${a.email}`}
              title={t("accounts.remove")}
              onClick={() => setConfirming(a)}
              className="flex size-7 items-center justify-center rounded-md text-text-3 hover:bg-hover hover:text-account-red"
            >
              <Trash2 size={14} />
            </button>
          </div>
        ))}
      </Group>
      {confirming && (
        <div
          role="alertdialog"
          className="mb-4 rounded-xl border border-line bg-raised p-3 text-[12px]"
        >
          <p>{t("accounts.confirmRemove", { email: confirming.email })}</p>
          {error && <p className="mt-1 text-text-2">{error}</p>}
          <div className="mt-2.5 flex justify-end gap-1.5">
            <button
              type="button"
              onClick={() => {
                setConfirming(null);
                setError(null);
              }}
              className="h-7 rounded-md px-3 text-text-2 hover:bg-hover"
            >
              {t("accounts.cancel")}
            </button>
            <button
              type="button"
              autoFocus
              onClick={() => {
                removeAccount(confirming.id)
                  .then(() => setConfirming(null))
                  .catch((e: Error) => setError(e.message));
              }}
              className="h-7 rounded-md bg-account-red px-3 font-medium text-white"
            >
              {t("accounts.confirm")}
            </button>
          </div>
        </div>
      )}
      <div className="flex gap-2">
        <button
          type="button"
          onClick={() => setSetup("choose")}
          className="flex h-8 items-center gap-1.5 rounded-md bg-accent px-3 text-[13px] font-medium text-on-accent hover:bg-accent-hover"
        >
          <Plus size={14} aria-hidden />
          {t("accounts.add")}
        </button>
        {IN_TAURI && !IS_ANDROID && (
          <button
            type="button"
            onClick={() => setSetup("import")}
            className="h-8 rounded-md px-3 text-[13px] text-text-2 hover:bg-hover hover:text-text"
          >
            {t("setup.importName")}
          </button>
        )}
      </div>
    </>
  );
}

function MailTab() {
  const { t } = useTranslation();
  const p = usePrefs();
  return (
    <>
      <Group title={t("settingsWindow.reading")}>
        <Row label={t("settings.readable")} hint={t("settings.readableHint")}>
          <Switch checked={p.readable} onChange={p.setReadable} label={t("settings.readable")} />
        </Row>
        <Row label={t("settings.noReplyReadOnly")} hint={t("settings.noReplyReadOnlyHint")}>
          <Switch
            checked={p.noReplyReadOnly}
            onChange={p.setNoReplyReadOnly}
            label={t("settings.noReplyReadOnly")}
          />
        </Row>
        <Row label={t("settings.showUnsubscribe")} hint={t("settings.showUnsubscribeHint")}>
          <Switch
            checked={p.showUnsubscribe}
            onChange={p.setShowUnsubscribe}
            label={t("settings.showUnsubscribe")}
          />
        </Row>
        <Row label={t("settings.avatars")} hint={t("settings.avatarsHint")}>
          <Switch checked={p.avatars} onChange={p.setAvatars} label={t("settings.avatars")} />
        </Row>
      </Group>
      <Group title={t("settingsWindow.apps")}>
        <Row label={t("settings.maps")}>
          <Segmented
            label={t("settings.maps")}
            value={p.maps}
            onChange={p.setMaps}
            options={[
              { value: "apple", label: t("settings.appleMaps") },
              { value: "google", label: t("settings.googleMaps") },
            ]}
          />
        </Row>
        <Row label={t("settings.calendar")}>
          <Segmented
            label={t("settings.calendar")}
            value={p.calendar}
            onChange={p.setCalendar}
            options={[
              { value: "apple", label: t("settings.appleCalendar") },
              { value: "google", label: t("settings.googleCalendar") },
            ]}
          />
        </Row>
      </Group>
    </>
  );
}

function PrivacyTab() {
  const { t } = useTranslation();
  const rules = useMail((s) => s.remoteRules);
  const remove = useMail((s) => s.removeRemoteRule);
  return (
    <Group title={t("remote.settingsTitle")} footer={t("settingsWindow.privacyFooter")}>
      {rules.length === 0 ? (
        <p className="px-3.5 py-3 text-[12px] text-text-2">{t("remote.none")}</p>
      ) : (
        rules.map((r) => (
          <div key={`${r.kind}:${r.value}`} className="flex items-center gap-3 px-3.5 py-2">
            <span className="w-24 shrink-0 text-[12px] text-text-2">
              {t(`remote.kind.${r.kind}`)}
            </span>
            <span className="min-w-0 flex-1 truncate text-[13px]" title={r.value}>
              {r.value}
            </span>
            <button
              type="button"
              aria-label={`${t("remote.remove")} ${r.value}`}
              title={t("remote.remove")}
              onClick={() => void remove(r)}
              className="flex size-7 items-center justify-center rounded-md text-text-3 hover:bg-hover hover:text-text"
            >
              <X size={14} />
            </button>
          </div>
        ))
      )}
    </Group>
  );
}

function StorageTab() {
  const { t } = useTranslation();
  const accounts = useMail((s) => s.accounts);
  const [usage, setUsage] = useState<StorageUsage | null>(null);
  const [phase, setPhase] = useState<"idle" | "confirm" | "running" | "done">("idle");
  const [errors, setErrors] = useState<string[]>([]);

  useEffect(() => {
    if (!IN_TAURI) return;
    void core
      .storageUsage()
      .then(setUsage)
      .catch(() => undefined);
  }, []);

  const reset = async () => {
    setPhase("running");
    setErrors([]);
    try {
      setUsage(await core.resetCache());
      const status = await core.syncStatus();
      setErrors(
        status.errors.map(
          (e) =>
            `${accounts.find((a) => a.id === e.accountId)?.label ?? e.accountId}: ${e.message}`,
        ),
      );
    } catch (e) {
      setErrors([(e as Error).message]);
    }
    setPhase("done");
  };

  const size = (bytes: number | undefined) => (bytes === undefined ? "–" : formatSize(bytes));
  const value = "shrink-0 text-[13px] text-text-2 tabular-nums";
  return (
    <>
      <Group title={t("settingsWindow.storageTitle")} footer={t("settingsWindow.storageFooter")}>
        <Row
          label={t("settingsWindow.storageMail")}
          hint={usage ? t("settingsWindow.storageMessages", { count: usage.messages }) : undefined}
        >
          <span className={value}>{size(usage?.database)}</span>
        </Row>
        <Row label={t("settingsWindow.storageAttachments")}>
          <span className={value}>{size(usage?.attachments)}</span>
        </Row>
        <Row label={t("settingsWindow.storageAvatars")}>
          <span className={value}>{size(usage?.avatars)}</span>
        </Row>
        <Row label={t("settingsWindow.storageTotal")}>
          <span className="shrink-0 text-[13px] font-medium tabular-nums">
            {size(usage ? usage.database + usage.attachments + usage.avatars : undefined)}
          </span>
        </Row>
      </Group>
      <Group>
        <Row
          label={t("settingsWindow.reset")}
          hint={t("settingsWindow.resetHint", { days: RESYNC_DAYS })}
        >
          <button
            type="button"
            disabled={!IN_TAURI || phase === "running" || phase === "confirm"}
            onClick={() => setPhase("confirm")}
            className="flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-line px-3 text-[12px] hover:bg-hover disabled:opacity-60 disabled:hover:bg-transparent"
          >
            {phase === "running" && <Loader2 size={13} className="animate-spin" aria-hidden />}
            {phase === "running" ? t("settingsWindow.resetting") : t("settingsWindow.resetButton")}
          </button>
        </Row>
      </Group>
      {phase === "confirm" && (
        <div
          role="alertdialog"
          className="mb-4 rounded-xl border border-line bg-raised p-3 text-[12px]"
        >
          <p>{t("settingsWindow.resetConfirm", { days: RESYNC_DAYS })}</p>
          <div className="mt-2.5 flex justify-end gap-1.5">
            <button
              type="button"
              onClick={() => setPhase("idle")}
              className="h-7 rounded-md px-3 text-text-2 hover:bg-hover"
            >
              {t("accounts.cancel")}
            </button>
            <button
              type="button"
              autoFocus
              onClick={() => void reset()}
              className="h-7 rounded-md bg-account-red px-3 font-medium text-white"
            >
              {t("settingsWindow.resetGo")}
            </button>
          </div>
        </div>
      )}
      {phase === "done" && (
        <div role="status" className="px-1 text-[12px] text-text-2">
          {errors.length === 0 ? (
            t("settingsWindow.resetDone", { days: RESYNC_DAYS })
          ) : (
            <>
              <p>{t("settingsWindow.resetFailed")}</p>
              <ul className="mt-1 list-disc pl-4">
                {errors.map((e) => (
                  <li key={e}>{e}</li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </>
  );
}

/** The Settings window (opened from the gear button or ⌘,). */
/** With `onClose`, a page inside the main window, which then owns the dialogs and toasts. */
export function SettingsWindow({ onClose }: { onClose?: () => void }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState(initialSettingsTab);
  const setup = useMail((s) => s.setup);
  const editingAccount = useMail((s) => s.editingAccount);
  useTheme();
  useLanguage();
  usePrefsFromOtherWindows();

  useEffect(() => {
    const mail = useMail.getState();
    void mail.loadAccounts();
    void mail.loadRemoteRules();
    if (!IN_TAURI) return;
    const offs = [
      listen("accounts-changed", () => void useMail.getState().loadAccounts()),
      listen("rules-changed", () => void useMail.getState().loadRemoteRules()),
      listen<SettingsTab>("settings-tab", (e) => setTab(e.payload)),
    ];
    const onKey = (e: KeyboardEvent) => {
      // By physical key, so ⌘W also works on the Ukrainian layout.
      if ((e.metaKey && e.code === "KeyW") || (e.key === "Escape" && !useMail.getState().setup)) {
        e.preventDefault();
        if (!onClose) return void getCurrentWindow().close();
        // An open dialog takes this Escape; otherwise the page behind must not see it.
        if (document.querySelector('[role="dialog"]')) return;
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, Boolean(onClose));
    return () => {
      offs.forEach((p) => void p.then((off) => off()));
      window.removeEventListener("keydown", onKey, Boolean(onClose));
    };
  }, [onClose]);

  useEffect(() => {
    document.title = `${t("settings.title")} – ${t(`settingsWindow.tab.${tab}`)}`;
  }, [t, tab]);

  return (
    <div className="flex h-full flex-col bg-bg">
      <header
        data-tauri-drag-region
        className={`shrink-0 border-b border-line pb-2 ${onClose ? "pt-1" : "pt-7"}`}
      >
        {onClose && (
          <IconButton label={t("setup.back")} className="mb-1 ml-1.5" onClick={onClose}>
            <ChevronLeft size={20} />
          </IconButton>
        )}
        <nav
          role="tablist"
          aria-label={t("settings.title")}
          className="flex justify-center gap-1 max-[700px]:px-2"
        >
          {TABS.map(({ id, icon: Icon }) => {
            const on = tab === id;
            return (
              <button
                key={id}
                type="button"
                role="tab"
                aria-selected={on}
                onClick={() => setTab(id)}
                className={`flex w-[76px] flex-col items-center max-[700px]:w-auto max-[700px]:min-w-0 max-[700px]:flex-1 gap-0.5 rounded-lg py-1.5 text-[11px] ${
                  on ? "bg-selected text-accent-text" : "text-text-2 hover:bg-hover hover:text-text"
                }`}
              >
                <Icon size={20} strokeWidth={1.75} aria-hidden />
                {t(`settingsWindow.tab.${id}`)}
              </button>
            );
          })}
        </nav>
      </header>
      <main className="flex-1 overflow-y-auto px-6 py-5 max-[700px]:px-4">
        <div className="mx-auto max-w-[540px]">
          {tab === "general" && <GeneralTab />}
          {tab === "accounts" && <AccountsTab />}
          {tab === "mail" && <MailTab />}
          {tab === "privacy" && <PrivacyTab />}
          {tab === "storage" && <StorageTab />}
        </div>
      </main>
      {!onClose && setup && <AddAccountDialog />}
      {!onClose && editingAccount && <AccountSettingsDialog />}
      {!onClose && <Toast />}
    </div>
  );
}

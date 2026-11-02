import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, TriangleAlert, X } from "lucide-react";
import { useMail } from "../store";
import { core } from "../lib/core";
import { describeError } from "../lib/errors";
import type { Account, AccountColor, Server } from "../types";
import { AccountDot, ColorPicker, IconButton } from "./primitives";
import { Modal } from "./Modal";
import { EMAIL, Field, inputCls, ServerFields } from "./form";
import { CREDENTIAL, NO_AUTOFILL } from "../lib/inputProps";

const sameServer = (a: Server, b: Server) =>
  a.host === b.host && a.port === b.port && a.security === b.security;

export function AccountSettingsDialog() {
  const id = useMail((s) => s.editingAccount);
  const account = useMail((s) => s.accounts.find((a) => a.id === id));
  const setEditingAccount = useMail((s) => s.setEditingAccount);
  if (!account) return null;
  // Keyed so reopening for another account starts from its saved values.
  return (
    <SettingsForm key={account.id} account={account} onClose={() => setEditingAccount(null)} />
  );
}

function SettingsForm({ account, onClose }: { account: Account; onClose: () => void }) {
  const { t } = useTranslation();
  const accountUpdated = useMail((s) => s.accountUpdated);
  const removeAccount = useMail((s) => s.removeAccount);

  const [displayName, setDisplayName] = useState(account.displayName);
  const [label, setLabel] = useState(account.label);
  const [color, setColor] = useState<AccountColor>(account.color);
  const [username, setUsername] = useState(
    account.username === account.email ? "" : account.username,
  );
  const [password, setPassword] = useState("");
  const [aliases, setAliases] = useState(account.aliases);
  const [defaultFrom, setDefaultFrom] = useState(account.defaultFrom);
  const [newAlias, setNewAlias] = useState("");
  const [aliasInvalid, setAliasInvalid] = useState(false);
  const [imap, setImap] = useState(account.imap);
  const [smtp, setSmtp] = useState(account.smtp);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ title: string; detail?: string } | null>(null);
  const [smtpWarning, setSmtpWarning] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);

  const passwordAuth = account.auth === "password";
  // Gmail and Microsoft use fixed servers; only generic IMAP accounts can change them.
  const editableServers = passwordAuth && account.provider === "imap";

  const loginChanged =
    (username.trim() || account.email) !== account.username ||
    !sameServer(imap, account.imap) ||
    !sameServer(smtp, account.smtp) ||
    password.length > 0;
  const changed =
    loginChanged ||
    displayName.trim() !== account.displayName ||
    (label.trim() || account.label) !== account.label ||
    color !== account.color ||
    newAlias.trim().length > 0 ||
    aliases.join() !== account.aliases.join() ||
    defaultFrom !== account.defaultFrom;
  const serversValid = Boolean(imap.host.trim() && imap.port && smtp.host.trim() && smtp.port);
  const canSave = changed && serversValid && !busy;

  /** Moves what's typed into the list and returns the list; null if it isn't an address. */
  const addTypedAlias = () => {
    const address = newAlias.trim().toLowerCase();
    if (!address) return aliases;
    if (!EMAIL.test(address)) {
      setAliasInvalid(true);
      return null;
    }
    const next =
      address === account.email || aliases.includes(address) ? aliases : [...aliases, address];
    setAliases(next);
    setNewAlias("");
    setAliasInvalid(false);
    return next;
  };

  const removeAlias = (address: string) => {
    setAliases(aliases.filter((a) => a !== address));
    if (defaultFrom === address) setDefaultFrom(account.email);
  };

  const save = async () => {
    // Typed but not added yet: saving means it was wanted.
    const nextAliases = addTypedAlias();
    if (!nextAliases) return;
    setBusy(true);
    setError(null);
    try {
      const result = await core.updateAccount(account.id, {
        label: label.trim(),
        displayName: displayName.trim(),
        aliases: nextAliases,
        defaultFrom,
        color,
        username: username.trim(),
        imap,
        smtp,
        password: password || undefined,
      });
      accountUpdated(result.account);
      setPassword("");
      // Stay open so the warning is read; otherwise the save is done.
      if (result.smtpWarning) setSmtpWarning(result.smtpWarning);
      else onClose();
    } catch (e) {
      setError(describeError(t, account.provider, e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal onClose={onClose} label={t("accountSettings.title")} className="top-[8vh] w-[480px]">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (canSave) void save();
        }}
      >
        <div className="flex items-center gap-2.5 border-b border-line py-2.5 pr-2 pl-4">
          <AccountDot color={color} className="size-2.5" />
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-[15px] font-semibold">{t("accountSettings.title")}</h2>
            <p className="selectable truncate text-[12px] text-text-2">{account.email}</p>
          </div>
          <IconButton label={t("accountSettings.close")} onClick={onClose}>
            <X size={16} />
          </IconButton>
        </div>

        <div className="flex max-h-[62vh] flex-col gap-3 overflow-y-auto px-4 py-4">
          <div className="grid grid-cols-2 gap-3">
            <Field
              label={t("setup.displayName")}
              hint={t("setup.displayNameHint")}
              htmlFor="acct-name"
            >
              <input
                id="acct-name"
                autoFocus
                autoComplete="name"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                className={inputCls}
              />
            </Field>
            <Field label={t("setup.label")} hint={t("setup.labelHint")} htmlFor="acct-label">
              <input
                id="acct-label"
                {...NO_AUTOFILL}
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                placeholder={account.label}
                className={inputCls}
              />
            </Field>
          </div>

          <div className="flex flex-col gap-1">
            <span className="text-[12px] font-medium text-text-2">{t("setup.color")}</span>
            <ColorPicker value={color} onChange={setColor} />
          </div>

          <fieldset className="flex flex-col gap-1">
            <legend className="mb-1 text-[12px] font-medium text-text-2">
              {t("accountSettings.addresses")}
            </legend>
            {aliases.length > 0 && (
              <ul className="divide-y divide-line rounded-md border border-line">
                {[account.email, ...aliases].map((address) => (
                  <li key={address} className="flex h-8 items-center gap-1 pr-0.5 pl-2.5">
                    <label className="flex min-w-0 flex-1 items-center gap-2 text-[13px]">
                      <input
                        type="radio"
                        name="default-from"
                        checked={defaultFrom === address}
                        onChange={() => setDefaultFrom(address)}
                        className="shrink-0 accent-accent"
                      />
                      <span className="selectable truncate">{address}</span>
                    </label>
                    {address !== account.email && (
                      <IconButton
                        label={t("accountSettings.removeAddress", { address })}
                        onClick={() => removeAlias(address)}
                      >
                        <X size={14} />
                      </IconButton>
                    )}
                  </li>
                ))}
              </ul>
            )}
            <div className="flex gap-2">
              <input
                aria-label={t("accountSettings.newAddress")}
                {...NO_AUTOFILL}
                {...CREDENTIAL}
                value={newAlias}
                aria-invalid={aliasInvalid}
                onChange={(e) => {
                  setNewAlias(e.target.value);
                  setAliasInvalid(false);
                }}
                onKeyDown={(e) => {
                  // Enter here adds the address instead of saving the form.
                  if (e.key === "Enter") {
                    e.preventDefault();
                    addTypedAlias();
                  }
                }}
                placeholder={t("accountSettings.newAddress")}
                className={inputCls}
              />
              <button
                type="button"
                disabled={!newAlias.trim()}
                onClick={addTypedAlias}
                className="h-8 shrink-0 rounded-md px-3 text-[13px] text-text-2 hover:bg-hover hover:text-text disabled:opacity-40"
              >
                {t("accountSettings.addAddress")}
              </button>
            </div>
            <span
              role={aliasInvalid ? "alert" : undefined}
              className={`text-[11px] ${aliasInvalid ? "text-account-red" : "text-text-2"}`}
            >
              {aliasInvalid
                ? t("accountSettings.addressInvalid")
                : t(
                    account.provider === "gmail"
                      ? "accountSettings.addressesHintGmail"
                      : "accountSettings.addressesHint",
                  )}
            </span>
          </fieldset>

          {passwordAuth ? (
            <Field
              label={t(
                account.provider === "gmail"
                  ? "accountSettings.newAppPassword"
                  : "accountSettings.newPassword",
              )}
              hint={t("accountSettings.newPasswordHint")}
              htmlFor="acct-password"
            >
              <input
                id="acct-password"
                name="password"
                type="password"
                autoComplete="current-password"
                {...CREDENTIAL}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className={inputCls}
              />
            </Field>
          ) : (
            <p className="rounded-lg bg-surface px-3 py-2 text-[12px] text-text-2">
              {t("accountSettings.oauthNote", {
                provider: t(`setup.provider.${account.provider}`),
              })}
            </p>
          )}

          {editableServers && (
            <details className="rounded-lg border border-line">
              <summary className="cursor-default px-3 py-2 text-[12px] font-medium text-text-2 select-none">
                {t("setup.servers")}
              </summary>
              <div className="flex flex-col gap-3 border-t border-line px-3 py-3">
                <Field
                  label={t("setup.username")}
                  hint={t("setup.usernameHint")}
                  htmlFor="acct-username"
                >
                  <input
                    id="acct-username"
                    name="username"
                    autoComplete="username"
                    {...CREDENTIAL}
                    value={username}
                    onChange={(e) => setUsername(e.target.value)}
                    placeholder={account.email}
                    className={inputCls}
                  />
                </Field>
                <ServerFields
                  id="imap"
                  title={t("setup.incoming")}
                  value={imap}
                  onChange={setImap}
                  imapOnly
                />
                <ServerFields
                  id="smtp"
                  title={t("setup.outgoing")}
                  value={smtp}
                  onChange={setSmtp}
                />
              </div>
            </details>
          )}

          {error && (
            <div
              ref={(el) => el?.scrollIntoView({ block: "nearest" })}
              role="alert"
              className="rounded-lg bg-account-red/10 px-3 py-2 text-[12px]"
            >
              <div className="font-medium text-text">{error.title}</div>
              {error.detail && (
                <div className="selectable mt-0.5 text-text-2 [overflow-wrap:anywhere]">
                  {error.detail}
                </div>
              )}
            </div>
          )}

          {smtpWarning && (
            <div
              ref={(el) => el?.scrollIntoView({ block: "nearest" })}
              role="status"
              className="rounded-lg bg-surface px-3 py-2 text-[12px]"
            >
              <div className="flex items-center gap-1.5 font-medium">
                <TriangleAlert size={14} aria-hidden className="text-account-gold" />
                {t("accountSettings.savedSmtpWarning")}
              </div>
              <div className="selectable mt-1 text-text-2 [overflow-wrap:anywhere]">
                {smtpWarning}
              </div>
            </div>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-line px-4 py-2.5">
          {confirmRemove ? (
            <>
              <span role="alert" className="mr-auto min-w-0 text-[12px]">
                {t("accounts.confirmRemove", { email: account.email })}
              </span>
              <button
                type="button"
                onClick={() => setConfirmRemove(false)}
                className="h-7 shrink-0 rounded-md px-3 text-[13px] text-text-2 hover:bg-hover"
              >
                {t("accounts.cancel")}
              </button>
              <button
                type="button"
                autoFocus
                onClick={() => {
                  removeAccount(account.id).catch((e: unknown) => {
                    setConfirmRemove(false);
                    setError(describeError(t, account.provider, e));
                  });
                }}
                className="h-7 shrink-0 rounded-md bg-account-red px-3 text-[13px] font-medium text-white"
              >
                {t("accounts.confirm")}
              </button>
            </>
          ) : (
            <>
              {busy ? (
                <span
                  className="mr-auto flex items-center gap-1.5 text-[12px] text-text-2"
                  aria-live="polite"
                >
                  <Loader2 size={13} className="animate-spin" aria-hidden />
                  {t(loginChanged ? "setup.checking" : "accountSettings.saving")}
                </span>
              ) : (
                <button
                  type="button"
                  onClick={() => setConfirmRemove(true)}
                  className="mr-auto h-7 rounded-md px-2 text-[13px] text-account-red hover:bg-account-red/10"
                >
                  {t("accountSettings.remove")}
                </button>
              )}
              <button
                type="button"
                onClick={onClose}
                className="h-7 rounded-md px-3 text-[13px] text-text-2 hover:bg-hover"
              >
                {t(smtpWarning && !changed ? "setup.done" : "accounts.cancel")}
              </button>
              <button
                type="submit"
                disabled={!canSave}
                className="h-7 rounded-md bg-accent px-4 text-[13px] font-medium text-on-accent hover:bg-accent-hover disabled:opacity-40"
              >
                {t("accountSettings.save")}
              </button>
            </>
          )}
        </div>
      </form>
    </Modal>
  );
}

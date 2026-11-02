import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { endSignIn, openLink, openSignIn } from "../lib/openLink";
import {
  ArrowLeft,
  Building2,
  Check,
  ChevronRight,
  ExternalLink,
  Import,
  Loader2,
  Mail,
  Server as ServerIcon,
  TriangleAlert,
} from "lucide-react";
import { useMail } from "../store";
import { IS_ANDROID, IN_TAURI } from "../lib/core";
import { core, CoreError } from "../lib/core";
import {
  ACCOUNT_COLORS,
  type Account,
  type AccountColor,
  type BrowserStart,
  type ImportCandidate,
  type Provider,
  type Server,
} from "../types";
import { ColorPicker } from "./primitives";
import { EMAIL, Field, inputCls, ServerFields } from "./form";
import { CREDENTIAL, NO_AUTOFILL } from "../lib/inputProps";
import { describeError } from "../lib/errors";
import { Modal } from "./Modal";

const PROVIDERS: Provider[] = ["gmail", "microsoft", "imap"];
const PROVIDER_ICON = { gmail: Mail, microsoft: Building2, imap: ServerIcon } as const;

const PRESETS: Record<"gmail" | "microsoft", { imap: Server; smtp: Server }> = {
  gmail: {
    imap: { host: "imap.gmail.com", port: 993, security: "tls" },
    smtp: { host: "smtp.gmail.com", port: 465, security: "tls" },
  },
  microsoft: {
    imap: { host: "outlook.office365.com", port: 993, security: "tls" },
    smtp: { host: "smtp.office365.com", port: 587, security: "starttls" },
  },
};

/** Values a form starts with: an address carried over, or an account found in Apple Mail. */
interface Initial {
  email: string;
  username?: string;
  displayName?: string;
  aliases?: string[];
  label?: string;
  imap?: Server | null;
  smtp?: Server | null;
  fromMail?: boolean;
}

function fromCandidate(c: ImportCandidate): Initial {
  return {
    email: c.email,
    username: c.username === c.email ? "" : c.username,
    displayName: c.displayName,
    aliases: c.aliases,
    label: c.name,
    imap: c.imap,
    smtp: c.smtp,
    fromMail: true,
  };
}

const EMPTY_SERVER: Server = { host: "", port: 993, security: "tls" };
const GMAIL_APP_PASSWORDS = "https://myaccount.google.com/apppasswords";

function domainOf(email: string): string {
  return email.split("@")[1]?.toLowerCase() ?? "";
}

function nextColor(accounts: Account[]): AccountColor {
  const used = new Set(accounts.map((a) => a.color));
  return (
    ACCOUNT_COLORS.find((c) => !used.has(c)) ??
    ACCOUNT_COLORS[accounts.length % ACCOUNT_COLORS.length]
  );
}

/** Provider rows; shared by the dialog and the first-run screen. */
export function ProviderList({ onPick }: { onPick: (p: Provider | "import") => void }) {
  const { t } = useTranslation();
  const rows = [
    // Reading Mail's settings needs the macOS core.
    ...(IN_TAURI && !IS_ANDROID
      ? [
          {
            id: "import" as const,
            Icon: Import,
            title: t("setup.importName"),
            desc: t("setup.importDesc"),
          },
        ]
      : []),
    ...PROVIDERS.map((p) => ({
      id: p,
      Icon: PROVIDER_ICON[p],
      title: t(`setup.provider.${p}`),
      desc: t(`setup.providerDesc.${p}`),
    })),
  ];
  return (
    <ul className="flex flex-col gap-1">
      {rows.map(({ id, Icon, title, desc }) => (
        <li key={id}>
          <button
            type="button"
            onClick={() => onPick(id)}
            className="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left hover:bg-hover"
          >
            <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-surface text-text-2">
              <Icon size={18} aria-hidden />
            </span>
            <span className="min-w-0 flex-1">
              <span className="block text-[13px] font-medium">{title}</span>
              <span className="block text-[12px] text-text-2">{desc}</span>
            </span>
            <ChevronRight size={16} className="shrink-0 text-text-3" aria-hidden />
          </button>
        </li>
      ))}
    </ul>
  );
}

function ImportList({
  onPick,
  onBack,
}: {
  onPick: (c: ImportCandidate) => void;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  const [state, setState] = useState<
    | { step: "loading" }
    | { step: "list"; items: ImportCandidate[] }
    | { step: "failed"; code: string; title: string; detail?: string }
  >({ step: "loading" });

  useEffect(() => {
    let live = true;
    core
      .listMailAppAccounts()
      .then((items) => live && setState({ step: "list", items }))
      .catch(
        (e: unknown) =>
          live &&
          setState({
            step: "failed",
            code: e instanceof CoreError ? e.code : "generic",
            ...describeError(t, "imap", e),
          }),
      );
    return () => {
      live = false;
    };
  }, [t]);

  return (
    <div>
      <DialogHeader title={t("setup.importTitle")} onBack={onBack} />
      <div className="max-h-[62vh] overflow-y-auto px-4 py-3">
        {state.step === "loading" && (
          <p className="flex items-center gap-1.5 py-6 text-[12px] text-text-2" aria-live="polite">
            <Loader2 size={13} className="animate-spin" aria-hidden />
            {t("setup.importLoading")}
          </p>
        )}
        {state.step === "failed" && (
          <div role="alert" className="rounded-lg bg-account-red/10 px-3 py-2 text-[12px]">
            <div className="font-medium">{state.title}</div>
            {state.detail && (
              <div className="selectable mt-0.5 text-text-2 [overflow-wrap:anywhere]">
                {state.detail}
              </div>
            )}
            {state.code === "mail_not_allowed" && (
              <button
                type="button"
                onClick={() =>
                  void openUrl(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation",
                  )
                }
                className="mt-2 inline-flex items-center gap-1 font-medium text-accent-text hover:underline"
              >
                {t("setup.openSettings")}
                <ExternalLink size={12} aria-hidden />
              </button>
            )}
          </div>
        )}
        {state.step === "list" && (
          <>
            <p className="mb-2 text-[12px] text-text-2">
              {state.items.length ? t("setup.importHint") : t("setup.importEmpty")}
            </p>
            <ul className="-mx-1 flex flex-col">
              {state.items.map((c) => {
                const Icon = PROVIDER_ICON[c.provider];
                return (
                  <li key={c.email}>
                    <button
                      type="button"
                      disabled={c.alreadyConnected}
                      onClick={() => onPick(c)}
                      className="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left hover:bg-hover disabled:hover:bg-transparent"
                    >
                      <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-surface text-text-2">
                        <Icon size={16} aria-hidden />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[13px] font-medium">{c.name}</span>
                        <span className="block truncate text-[12px] text-text-2">
                          {c.email}
                          {c.aliases.length > 0 &&
                            ` · ${t("setup.importAliases", { count: c.aliases.length })}`}
                        </span>
                        <span className="block truncate text-[11px] text-text-2">
                          {t(`setup.provider.${c.provider}`)}
                          {c.provider === "imap" && c.imap && ` · ${c.imap.host}`}
                        </span>
                      </span>
                      {c.alreadyConnected ? (
                        <span className="flex shrink-0 items-center gap-1 text-[12px] text-done">
                          <Check size={13} aria-hidden />
                          {t("setup.importConnected")}
                        </span>
                      ) : (
                        <ChevronRight size={16} className="shrink-0 text-text-3" aria-hidden />
                      )}
                    </button>
                  </li>
                );
              })}
            </ul>
          </>
        )}
      </div>
    </div>
  );
}

function DialogHeader({ title, onBack }: { title: string; onBack: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-2 border-b border-line px-3 py-2.5">
      <button
        type="button"
        onClick={onBack}
        aria-label={t("setup.back")}
        title={t("setup.back")}
        className="flex size-7 items-center justify-center rounded-md text-text-2 hover:bg-hover"
      >
        <ArrowLeft size={16} />
      </button>
      <h2 className="text-[15px] font-semibold">{title}</h2>
    </div>
  );
}

export function AddAccountDialog() {
  const { t } = useTranslation();
  const setup = useMail((s) => s.setup);
  const setSetup = useMail((s) => s.setSetup);
  const [initial, setInitial] = useState<Initial>({ email: "" });
  const [fromImport, setFromImport] = useState(false);
  const [connected, setConnected] = useState<{
    account: Account;
    smtpWarning: string | null;
  } | null>(null);
  const close = () => setSetup(null);

  if (setup === null) return null;

  return (
    <Modal onClose={close} label={t("setup.title")} className="top-[8vh] w-[520px]">
      {connected ? (
        <Connected
          {...connected}
          onClose={close}
          onNext={
            fromImport
              ? () => {
                  setConnected(null);
                  setSetup("import");
                }
              : undefined
          }
        />
      ) : setup === "choose" ? (
        <div className="p-4">
          <h2 className="text-[17px] font-semibold">{t("setup.chooseTitle")}</h2>
          <p className="mt-1 mb-3 text-[12px] text-text-2">{t("setup.chooseHint")}</p>
          <ProviderList
            onPick={(p) => {
              setFromImport(p === "import");
              setInitial({ email: "" });
              setSetup(p);
            }}
          />
        </div>
      ) : setup === "import" ? (
        <ImportList
          onBack={() => setSetup("choose")}
          onPick={(c) => {
            setInitial(fromCandidate(c));
            setSetup(c.provider);
          }}
        />
      ) : (
        <AccountForm
          key={`${setup}:${initial.email}`}
          provider={setup}
          initial={initial}
          onBack={() => setSetup(fromImport ? "import" : "choose")}
          onSwitch={(p, email) => {
            setInitial({ ...initial, email, imap: null, smtp: null, fromMail: false });
            setSetup(p);
          }}
          onConnected={setConnected}
        />
      )}
    </Modal>
  );
}

function Connected({
  account,
  smtpWarning,
  onClose,
  onNext,
}: {
  account: Account;
  smtpWarning: string | null;
  onClose: () => void;
  /** Back to the Apple Mail list to connect the next account. */
  onNext?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col items-center px-6 pt-8 pb-5 text-center">
      <span className="flex size-10 items-center justify-center rounded-full bg-done/15 text-done">
        <Check size={22} aria-hidden />
      </span>
      <h2 className="mt-3 text-[17px] font-semibold">
        {t("setup.connectedTitle", { email: account.email })}
      </h2>
      {smtpWarning && (
        <div className="mt-3 w-full rounded-lg bg-surface px-3 py-2 text-left text-[12px]">
          <div className="flex items-center gap-1.5 font-medium">
            <TriangleAlert size={14} aria-hidden className="text-account-gold" />
            {t("setup.smtpWarning")}
          </div>
          <div className="selectable mt-1 text-text-2 [overflow-wrap:anywhere]">{smtpWarning}</div>
        </div>
      )}
      <p className="mt-3 text-[12px] text-text-2">{t("setup.syncNote")}</p>
      <div className="mt-5 flex gap-2">
        {onNext && (
          <button
            type="button"
            onClick={onNext}
            className="h-8 rounded-md px-4 text-[13px] text-text-2 hover:bg-hover"
          >
            {t("setup.importNext")}
          </button>
        )}
        <button
          type="button"
          autoFocus
          onClick={onClose}
          className="h-8 rounded-md bg-accent px-5 text-[13px] font-medium text-on-accent hover:bg-accent-hover"
        >
          {t("setup.done")}
        </button>
      </div>
    </div>
  );
}

function AccountForm({
  provider,
  initial,
  onBack,
  onSwitch,
  onConnected,
}: {
  provider: Provider;
  initial: Initial;
  onBack: () => void;
  onSwitch: (p: Provider, email: string) => void;
  onConnected: (r: { account: Account; smtpWarning: string | null }) => void;
}) {
  const { t } = useTranslation();
  const accounts = useMail((s) => s.accounts);
  const accountAdded = useMail((s) => s.accountAdded);

  const [email, setEmail] = useState(initial.email);
  const [username, setUsername] = useState(initial.username ?? "");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState(
    initial.displayName || accounts[0]?.displayName || "",
  );
  const [label, setLabel] = useState(initial.label ?? (provider === "gmail" ? "Gmail" : ""));
  const [color, setColor] = useState<AccountColor>(() => nextColor(accounts));
  const preset = provider === "imap" ? null : PRESETS[provider];
  const [imap, setImap] = useState<Server>(initial.imap ?? preset?.imap ?? EMPTY_SERVER);
  const [smtp, setSmtp] = useState<Server>(
    initial.smtp ?? preset?.smtp ?? { ...EMPTY_SERVER, port: 465 },
  );
  const prefilled = provider === "imap" && Boolean(initial.imap && initial.smtp);
  const [detect, setDetect] = useState<"idle" | "detecting" | "found" | "missing">(
    prefilled ? "found" : "idle",
  );
  const [hostedBy, setHostedBy] = useState<Provider | null>(null);
  // Open when Mail gave a login that differs from the address, so it is visible.
  const [showServers, setShowServers] = useState(provider === "imap" && Boolean(initial.username));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ title: string; detail?: string } | null>(null);
  const lastDetected = useRef(prefilled ? initial.email : "");
  // Gmail: Sign in with Google when this build has an OAuth client, app password otherwise.
  const [googleAvailable, setGoogleAvailable] = useState<boolean | null>(
    provider === "gmail" ? null : false,
  );
  const [usePassword, setUsePassword] = useState(false);
  const passwordMode =
    provider === "imap" || (provider === "gmail" && (googleAvailable === false || usePassword));

  useEffect(() => {
    if (provider === "gmail") void core.googleOauthAvailable().then(setGoogleAvailable);
  }, [provider]);

  const emailValid = EMAIL.test(email.trim());
  const domain = domainOf(email.trim());

  const runDetect = async () => {
    const addr = email.trim();
    if (provider !== "imap" || !EMAIL.test(addr) || lastDetected.current === addr) return;
    lastDetected.current = addr;
    setDetect("detecting");
    setHostedBy(null);
    try {
      const d = await core.discoverServers(addr);
      if (lastDetected.current !== addr) return;
      if (d.provider !== "imap") setHostedBy(d.provider);
      if (d.imap) setImap(d.imap);
      if (d.smtp) setSmtp(d.smtp);
      const found = Boolean(d.imap && d.smtp);
      setDetect(found ? "found" : "missing");
      if (!found) setShowServers(true);
    } catch {
      setDetect("missing");
      setShowServers(true);
    }
  };

  const draft = () => ({
    provider,
    label: label.trim() || (provider === "gmail" ? "Gmail" : domain),
    email: email.trim(),
    username: username.trim(),
    displayName: displayName.trim(),
    aliases: initial.aliases ?? [],
    color,
    imap,
    smtp,
  });

  const finish = async (secret: Parameters<typeof core.addAccount>[1]) => {
    setBusy(true);
    setError(null);
    try {
      const result = await core.addAccount(draft(), secret);
      accountAdded(result.account);
      onConnected(result);
    } catch (e) {
      // The Gmail "use an app password" hint does not apply after Sign in with Google.
      const hintFor = secret.kind === "oauth" && provider === "gmail" ? "imap" : provider;
      setError(describeError(t, hintFor, e));
    } finally {
      setBusy(false);
    }
  };

  // The login password managers fill is the address, unless a separate username field is shown.
  const usernameShown = provider === "imap" ? showServers : Boolean(initial.username);

  const serversValid = Boolean(imap.host.trim() && imap.port && smtp.host.trim() && smtp.port);
  const canSubmit = passwordMode && emailValid && password.length > 0 && serversValid && !busy;

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (canSubmit) void finish({ kind: "password", password });
      }}
    >
      <DialogHeader title={t(`setup.provider.${provider}`)} onBack={onBack} />

      <div className="flex max-h-[62vh] flex-col gap-3 overflow-y-auto px-4 py-4">
        {provider === "gmail" && passwordMode && (
          <div className="rounded-lg bg-surface px-3 py-2.5 text-[12px] leading-relaxed text-text-2">
            {t("setup.gmailHelp")}{" "}
            <button
              type="button"
              onClick={() => openLink(GMAIL_APP_PASSWORDS)}
              className="inline-flex items-center gap-1 font-medium text-accent-text hover:underline"
            >
              {t("setup.gmailHelpLink")}
              <ExternalLink size={12} aria-hidden />
            </button>
          </div>
        )}

        <Field label={t("setup.email")} htmlFor="setup-email">
          <input
            id="setup-email"
            name="email"
            type="email"
            autoFocus
            autoComplete={usernameShown ? "email" : "username"}
            {...CREDENTIAL}
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            onBlur={() => void runDetect()}
            placeholder={provider === "gmail" ? "name@gmail.com" : "name@example.com"}
            className={inputCls}
          />
        </Field>

        {provider === "imap" && detect !== "idle" && (
          <div className="flex items-center gap-1.5 text-[12px] text-text-2" aria-live="polite">
            {detect === "detecting" && <Loader2 size={13} className="animate-spin" aria-hidden />}
            {detect === "found" && <Check size={13} className="text-done" aria-hidden />}
            {detect === "detecting"
              ? t("setup.detecting")
              : detect === "found"
                ? `${initial.fromMail && email.trim() === initial.email ? t("setup.fromMail") : t("setup.detected", { domain })} · ${imap.host}`
                : t("setup.notDetected")}
          </div>
        )}

        {hostedBy && hostedBy !== provider && (
          <div className="flex items-center justify-between gap-3 rounded-lg bg-surface px-3 py-2 text-[12px]">
            <span>{t("setup.hostedBy", { provider: t(`setup.provider.${hostedBy}`) })}</span>
            <button
              type="button"
              onClick={() => onSwitch(hostedBy, email.trim())}
              className="shrink-0 font-medium text-accent-text hover:underline"
            >
              {t("setup.useConnector", { provider: t(`setup.provider.${hostedBy}`) })}
            </button>
          </div>
        )}

        {provider !== "imap" && initial.username && (
          <Field
            label={t("setup.username")}
            hint={t("setup.usernameHint")}
            htmlFor="setup-username"
          >
            <input
              id="setup-username"
              name="username"
              autoComplete="username"
              {...CREDENTIAL}
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder={email.trim() || undefined}
              className={inputCls}
            />
          </Field>
        )}

        {passwordMode && (
          <Field
            label={t(provider === "gmail" ? "setup.appPassword" : "setup.password")}
            htmlFor="setup-password"
          >
            <input
              id="setup-password"
              name="password"
              type="password"
              autoComplete="current-password"
              {...CREDENTIAL}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={provider === "gmail" ? "xxxx xxxx xxxx xxxx" : undefined}
              className={inputCls}
            />
          </Field>
        )}

        <div className="grid grid-cols-2 gap-3">
          <Field
            label={t("setup.displayName")}
            hint={t("setup.displayNameHint")}
            htmlFor="setup-name"
          >
            <input
              id="setup-name"
              autoComplete="name"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              className={inputCls}
            />
          </Field>
          <Field label={t("setup.label")} hint={t("setup.labelHint")} htmlFor="setup-label">
            <div className="flex items-center gap-2">
              <input
                id="setup-label"
                {...NO_AUTOFILL}
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                placeholder={domain || undefined}
                className={inputCls}
              />
            </div>
          </Field>
        </div>

        <ColorPicker value={color} onChange={setColor} />

        {provider === "imap" && (
          <details
            open={showServers}
            onToggle={(e) => setShowServers((e.target as HTMLDetailsElement).open)}
            className="rounded-lg border border-line"
          >
            <summary className="cursor-default px-3 py-2 text-[12px] font-medium text-text-2 select-none">
              {t("setup.servers")}
            </summary>
            <div className="flex flex-col gap-3 border-t border-line px-3 py-3">
              <Field
                label={t("setup.username")}
                hint={t("setup.usernameHint")}
                htmlFor="setup-username"
              >
                <input
                  id="setup-username"
                  name="username"
                  autoComplete="username"
                  {...CREDENTIAL}
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  placeholder={email.trim() || undefined}
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
              <ServerFields id="smtp" title={t("setup.outgoing")} value={smtp} onChange={setSmtp} />
            </div>
          </details>
        )}

        {provider === "gmail" && googleAvailable && !usePassword && (
          <BrowserSignIn
            provider="gmail"
            label={t("setup.googleSignIn")}
            waitingNote={t("setup.googleNote")}
            begin={() => core.googleOauthStart(email.trim())}
            disabled={!emailValid || busy}
            busy={busy}
            failed={Boolean(error)}
            onSignedIn={(session) => void finish({ kind: "oauth", session })}
          />
        )}

        {provider === "gmail" && googleAvailable && (
          <button
            type="button"
            onClick={() => {
              setUsePassword(!usePassword);
              setError(null);
            }}
            className="self-start text-[12px] text-accent-text hover:underline"
          >
            {t(usePassword ? "setup.googleUseOauth" : "setup.googleUsePassword")}
          </button>
        )}

        {provider === "microsoft" && (
          <MicrosoftSignIn
            email={email.trim()}
            disabled={!emailValid || busy}
            busy={busy}
            failed={Boolean(error)}
            onSignedIn={(session) => void finish({ kind: "oauth", session })}
          />
        )}

        {error && (
          <div role="alert" className="rounded-lg bg-account-red/10 px-3 py-2 text-[12px]">
            <div className="font-medium text-text">{error.title}</div>
            {error.detail && (
              <div className="selectable mt-0.5 text-text-2 [overflow-wrap:anywhere]">
                {error.detail}
              </div>
            )}
          </div>
        )}
      </div>

      {passwordMode && (
        <div className="flex items-center justify-end gap-2 border-t border-line px-4 py-2.5">
          {busy && (
            <span
              className="mr-auto flex items-center gap-1.5 text-[12px] text-text-2"
              aria-live="polite"
            >
              <Loader2 size={13} className="animate-spin" aria-hidden />
              {t("setup.checking")}
            </span>
          )}
          <button
            type="button"
            onClick={onBack}
            className="h-7 rounded-md px-3 text-[13px] text-text-2 hover:bg-hover"
          >
            {t("setup.back")}
          </button>
          <button
            type="submit"
            disabled={!canSubmit}
            className="h-7 rounded-md bg-accent px-4 text-[13px] font-medium text-on-accent hover:bg-accent-hover disabled:opacity-40"
          >
            {t("setup.connect")}
          </button>
        </div>
      )}
    </form>
  );
}

type SignInState =
  | { step: "idle" }
  | { step: "browser"; url: string }
  | { step: "signedIn" }
  | { step: "failed"; title: string; detail?: string };

/** Opens the provider's sign-in page and waits for the loopback redirect handled by the core. */
function BrowserSignIn({
  provider,
  label,
  idleNote,
  waitingNote,
  begin,
  disabled,
  busy,
  failed,
  onSignedIn,
}: {
  provider: Provider;
  label: string;
  idleNote?: string;
  waitingNote: string;
  begin: () => Promise<BrowserStart>;
  disabled: boolean;
  busy: boolean;
  /** The mailbox check after sign-in failed; offer to start over. */
  failed: boolean;
  onSignedIn: (session: string) => void;
}) {
  const { t } = useTranslation();
  const [state, setState] = useState<SignInState>({ step: "idle" });
  const run = useRef(0);

  useEffect(() => {
    const polling = run;
    return () => {
      polling.current++; // stops any polling loop
      endSignIn();
    };
  }, []);

  const start = async () => {
    const id = ++run.current;
    try {
      const b = await begin();
      setState({ step: "browser", url: b.url });
      await openSignIn(b.url);
      while (run.current === id) {
        await new Promise((r) => setTimeout(r, 1000));
        if (run.current !== id) return;
        try {
          await core.oauthPoll(b.session);
          setState({ step: "signedIn" });
          onSignedIn(b.session);
          return;
        } catch (e) {
          if (e instanceof CoreError && e.code === "oauth_pending") continue;
          throw e;
        }
      }
    } catch (e) {
      if (run.current === id) setState({ step: "failed", ...describeError(t, provider, e) });
    } finally {
      if (run.current === id) endSignIn();
    }
  };

  if (state.step === "browser") {
    return (
      <div className="flex flex-col gap-1.5 rounded-lg border border-line px-3 py-3 text-[12px]">
        <p className="flex items-center gap-1.5 text-text-2" aria-live="polite">
          <Loader2 size={13} className="animate-spin" aria-hidden />
          {t("setup.browserWaiting")}
        </p>
        <button
          type="button"
          onClick={() => void openSignIn(state.url)}
          className="inline-flex items-center gap-1 self-start font-medium text-accent-text hover:underline"
        >
          {t("setup.browserReopen")}
          <ExternalLink size={12} aria-hidden />
        </button>
        <p className="text-[11px] text-text-2">{waitingNote}</p>
      </div>
    );
  }

  if (state.step === "signedIn" && !(failed && !busy)) {
    return (
      <p className="flex items-center gap-1.5 text-[12px] text-text-2" aria-live="polite">
        <Loader2 size={13} className={busy ? "animate-spin" : "hidden"} aria-hidden />
        {t("setup.browserSignedIn")}
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {state.step === "failed" && (
        <div role="alert" className="rounded-lg bg-account-red/10 px-3 py-2 text-[12px]">
          <div className="font-medium">{state.title}</div>
          {state.detail && (
            <div className="selectable mt-0.5 text-text-2 [overflow-wrap:anywhere]">
              {state.detail}
            </div>
          )}
        </div>
      )}
      <button
        type="button"
        disabled={disabled}
        onClick={() => void start()}
        className="h-8 self-start rounded-md bg-accent px-4 text-[13px] font-medium text-on-accent hover:bg-accent-hover disabled:opacity-40"
      >
        {state.step === "idle" ? label : t("setup.signInRetry")}
      </button>
      {idleNote && <p className="text-[11px] text-text-2">{idleNote}</p>}
    </div>
  );
}

function MicrosoftSignIn({
  email,
  disabled,
  busy,
  failed,
  onSignedIn,
}: {
  email: string;
  disabled: boolean;
  busy: boolean;
  failed: boolean;
  onSignedIn: (session: string) => void;
}) {
  const { t } = useTranslation();
  const [available, setAvailable] = useState<boolean | null>(null);

  useEffect(() => {
    void core.msOauthAvailable().then(setAvailable);
  }, []);

  if (available === false) {
    return (
      <div className="rounded-lg bg-surface px-3 py-2.5 text-[12px] leading-relaxed text-text-2">
        {t("setup.msNotConfigured")}
      </div>
    );
  }

  return (
    <BrowserSignIn
      provider="microsoft"
      label={t("setup.msSignIn")}
      idleNote={t("setup.msOrgNote")}
      waitingNote={t("setup.msOrgNote")}
      begin={() => core.msOauthStart(email)}
      disabled={disabled || available === null}
      busy={busy}
      failed={failed}
      onSignedIn={onSignedIn}
    />
  );
}

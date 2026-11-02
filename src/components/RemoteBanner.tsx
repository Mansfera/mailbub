import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AtSign, Building2, Check, ChevronDown, ImageOff, Images } from "lucide-react";
import { useMail } from "../store";
import { domainMatches } from "../lib/emailHtml";
import type { Message, RemoteRule } from "../types";

type SenderScope = "sender" | "domain" | null;

/**
 * Shown above a message whose remote images/fonts are blocked. "Always…" lets the user
 * trust the sender or their whole domain (side by side, at most one), and additionally
 * allow individual image sources that are not covered by that choice.
 */
export function RemoteBanner({ message, blocked }: { message: Message; blocked: string[] }) {
  const { t } = useTranslation();
  const allowOnce = useMail((s) => s.allowRemoteOnce);
  const addRules = useMail((s) => s.addRemoteRules);
  const sender = message.from.address.toLowerCase();
  const senderDomain = sender.split("@")[1] ?? "";
  const [open, setOpen] = useState(false);
  const [scope, setScope] = useState<SenderScope>("sender");
  const [hosts, setHosts] = useState<Set<string>>(() => new Set());

  const rules: RemoteRule[] = [
    ...(scope === "sender" ? [{ kind: "sender", value: sender } as RemoteRule] : []),
    ...(scope === "domain" ? [{ kind: "domain", value: senderDomain } as RemoteRule] : []),
    ...[...hosts].map((value) => ({ kind: "host", value }) as RemoteRule),
  ];
  const save = () => void addRules(rules).then(() => setOpen(false));

  const cards: {
    id: Exclude<SenderScope, null>;
    icon: typeof AtSign;
    title: string;
    detail: string;
  }[] = [
    { id: "sender", icon: AtSign, title: t("remote.scope.sender"), detail: sender },
    ...(senderDomain
      ? [
          {
            id: "domain" as const,
            icon: Building2,
            title: t("remote.scope.domain", { domain: senderDomain }),
            detail: t("remote.scope.domainDetail"),
          },
        ]
      : []),
  ];

  return (
    <div className="overflow-hidden rounded-xl border border-line bg-raised text-[12px] text-text">
      <div className="flex items-center gap-3 px-3 py-2.5 max-[700px]:flex-wrap max-[700px]:gap-y-2">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-surface text-text-2">
          <ImageOff size={16} aria-hidden />
        </span>
        <div className="min-w-0 flex-1 max-[700px]:basis-[calc(100%-44px)]">
          <div className="font-medium">{t("remote.blocked")}</div>
          <div className="text-text-2">{t("remote.blockedHint")}</div>
        </div>
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className={`flex h-7 shrink-0 items-center gap-1 rounded-md px-2.5 whitespace-nowrap max-[700px]:ml-auto ${open ? "bg-hover text-text" : "text-text-2 hover:bg-hover hover:text-text"}`}
        >
          {t("remote.always")}
          <ChevronDown
            size={13}
            aria-hidden
            className={`transition-transform ${open ? "rotate-180" : ""}`}
          />
        </button>
        <button
          type="button"
          onClick={() => allowOnce(message.id)}
          className="h-7 shrink-0 rounded-md bg-accent px-3 font-medium text-on-accent hover:bg-accent-hover"
        >
          {t("remote.load")}
        </button>
      </div>

      {open && (
        <div className="border-t border-line bg-surface/60 px-3 py-3">
          <p className="mb-1.5 text-text-2">{t("remote.alwaysTitle")}</p>
          <div
            role="radiogroup"
            aria-label={t("remote.alwaysTitle")}
            className="grid grid-cols-2 gap-1.5 max-[700px]:grid-cols-1"
          >
            {cards.map(({ id, icon: Icon, title, detail }) => {
              const on = scope === id;
              return (
                <button
                  key={id}
                  type="button"
                  role="radio"
                  aria-checked={on}
                  // Clicking the chosen card again clears it, so only sources can be picked.
                  onClick={() => setScope(on ? null : id)}
                  className={`flex min-w-0 items-start gap-2 rounded-lg border px-2.5 py-2 text-left ${
                    on ? "border-brand bg-raised" : "border-line hover:bg-hover"
                  }`}
                >
                  <Icon
                    size={15}
                    aria-hidden
                    className={`mt-px shrink-0 ${on ? "text-accent-text" : "text-text-2"}`}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium" title={title}>
                      {title}
                    </span>
                    <span className="block truncate text-text-2" title={detail}>
                      {detail}
                    </span>
                  </span>
                  <span
                    aria-hidden
                    className={`mt-px flex size-4 shrink-0 items-center justify-center rounded-full border ${
                      on ? "border-brand bg-accent text-on-accent" : "border-line"
                    }`}
                  >
                    {on && <Check size={11} strokeWidth={3} />}
                  </span>
                </button>
              );
            })}
          </div>

          {blocked.length > 0 && (
            <div className="mt-3">
              <p className="mb-1.5 flex items-center gap-1.5 text-text-2">
                <Images size={13} aria-hidden />
                {t(scope ? "remote.alsoHosts" : "remote.onlyHosts")}
              </p>
              <div className="flex flex-wrap gap-1.5">
                {blocked.map((d) => {
                  const on = hosts.has(d);
                  // Already covered when the sender's whole domain is trusted.
                  const covered = scope === "domain" && domainMatches(d, senderDomain);
                  return (
                    <button
                      key={d}
                      type="button"
                      aria-pressed={on || covered}
                      disabled={covered}
                      onClick={() => {
                        const next = new Set(hosts);
                        if (on) next.delete(d);
                        else next.add(d);
                        setHosts(next);
                      }}
                      className={`flex h-6 items-center gap-1 rounded-full border px-2.5 disabled:opacity-50 ${
                        on || covered
                          ? "border-brand bg-brand/10 text-accent-text"
                          : "border-line text-text-2 hover:bg-hover"
                      }`}
                    >
                      {on || covered ? (
                        <Check size={11} strokeWidth={3} aria-hidden />
                      ) : (
                        <span aria-hidden>+</span>
                      )}
                      {d}
                    </button>
                  );
                })}
              </div>
            </div>
          )}

          <div className="mt-3 flex justify-end gap-1.5">
            <button
              type="button"
              onClick={() => setOpen(false)}
              className="h-7 rounded-md px-2.5 text-text-2 hover:bg-hover"
            >
              {t("remote.cancel")}
            </button>
            <button
              type="button"
              disabled={rules.length === 0}
              onClick={save}
              className="h-7 rounded-md bg-accent px-3 font-medium text-on-accent hover:bg-accent-hover disabled:opacity-40"
            >
              {t("remote.save")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

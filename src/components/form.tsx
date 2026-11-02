import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CREDENTIAL, NO_AUTOFILL } from "../lib/inputProps";
import type { Server } from "../types";

export function Field({
  label,
  hint,
  htmlFor,
  children,
}: {
  label: string;
  hint?: string;
  htmlFor: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={htmlFor} className="text-[12px] font-medium text-text-2">
        {label}
      </label>
      {children}
      {hint && <span className="text-[11px] text-text-2">{hint}</span>}
    </div>
  );
}

export const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export const inputCls =
  "selectable h-8 w-full rounded-md border border-line bg-bg px-2.5 text-[13px] outline-none placeholder:text-text-3 focus:border-brand disabled:opacity-60";

export function ServerFields({
  id,
  title,
  value,
  onChange,
  imapOnly = false,
}: {
  id: string;
  title: string;
  value: Server;
  onChange: (s: Server) => void;
  imapOnly?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <fieldset className="flex flex-col gap-1.5">
      <legend className="mb-1 text-[12px] font-medium">{title}</legend>
      <div className="grid grid-cols-[1fr_72px_110px] gap-2">
        <input
          aria-label={`${title} ${t("setup.host")}`}
          {...NO_AUTOFILL}
          {...CREDENTIAL}
          value={value.host}
          onChange={(e) => onChange({ ...value, host: e.target.value.trim() })}
          placeholder={id === "imap" ? "imap.example.com" : "smtp.example.com"}
          className={inputCls}
        />
        <input
          id={`${id}-port`}
          aria-label={`${title} ${t("setup.port")}`}
          inputMode="numeric"
          {...NO_AUTOFILL}
          value={value.port || ""}
          onChange={(e) =>
            onChange({ ...value, port: Number(e.target.value.replace(/\D/g, "")) || 0 })
          }
          className={`${inputCls} tabular-nums`}
        />
        <select
          aria-label={`${title} ${t("setup.security")}`}
          value={value.security}
          disabled={imapOnly}
          onChange={(e) => {
            const security = e.target.value as Server["security"];
            onChange({ ...value, security, port: security === "tls" ? 465 : 587 });
          }}
          className={inputCls}
        >
          <option value="tls">{t("setup.tls")}</option>
          <option value="starttls">{t("setup.starttls")}</option>
        </select>
      </div>
      {imapOnly && <span className="text-[11px] text-text-2">{t("setup.imapTlsOnly")}</span>}
    </fieldset>
  );
}

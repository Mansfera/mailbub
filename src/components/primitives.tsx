import { Fragment, useState, type ButtonHTMLAttributes, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import { initials } from "../lib/chat";
import { IN_TAURI, IS_ANDROID } from "../lib/core";
import { usePrefs } from "../store";
import { ACCOUNT_COLORS, type AccountColor, type Person } from "../types";

const ACCOUNT_BG: Record<AccountColor, string> = {
  red: "bg-account-red",
  teal: "bg-account-teal",
  gold: "bg-account-gold",
  plum: "bg-account-plum",
  olive: "bg-account-olive",
  slate: "bg-account-slate",
};

export function AccountDot({ color, className = "" }: { color: AccountColor; className?: string }) {
  return (
    <span
      aria-hidden
      className={`inline-block size-2 shrink-0 rounded-full ${ACCOUNT_BG[color]} ${className}`}
    />
  );
}

export function ColorPicker({
  value,
  onChange,
}: {
  value: AccountColor;
  onChange: (c: AccountColor) => void;
}) {
  const { t } = useTranslation();
  return (
    <div role="radiogroup" aria-label={t("setup.color")} className="flex items-center gap-1.5">
      {ACCOUNT_COLORS.map((c) => (
        <button
          key={c}
          type="button"
          role="radio"
          aria-checked={value === c}
          aria-label={t(`colors.${c}`)}
          title={t(`colors.${c}`)}
          onClick={() => onChange(c)}
          className={`flex size-6 items-center justify-center rounded-full ${
            value === c ? "ring-2 ring-brand" : "hover:bg-hover"
          }`}
        >
          <AccountDot color={c} className="size-3" />
        </button>
      ))}
    </div>
  );
}

/** How each address's picture turned out, so remounts skip the fade-in or the request. */
const pictures = new Map<string, "photo" | "logo" | "none">();

/** The core's profile picture over the initials; small site icons sit padded as a logo. */
function Picture({ address }: { address: string }) {
  const enabled = usePrefs((s) => s.avatars);
  const [kind, setKind] = useState(() => pictures.get(address));
  if (!IN_TAURI || !enabled || kind === "none") return null;
  const settle = (k: "photo" | "logo" | "none") => {
    pictures.set(address, k);
    setKind(k);
  };
  return (
    <img
      src={convertFileSrc(address, "avatar")}
      alt=""
      draggable={false}
      onLoad={(e) => {
        const w = e.currentTarget.naturalWidth;
        settle(w > 0 && w < 96 ? "logo" : "photo");
      }}
      onError={() => settle("none")}
      className={`absolute inset-0 size-full rounded-full bg-white ${
        kind === undefined ? "opacity-0" : ""
      } ${kind === "logo" ? "object-contain p-[18%]" : "object-cover"}`}
    />
  );
}

/** Initials avatar with the profile picture on top when there is one; a group shows the
 * first two people overlapped. */
export function Avatar({
  people,
  size = 40,
  accountColor,
}: {
  people: Person[];
  size?: number;
  accountColor?: AccountColor;
}) {
  const group = people.length > 1;
  const inner = group ? Math.round(size * 0.66) : size;
  const font = Math.round(inner * (group ? 0.4 : 0.36));
  return (
    <div className="relative shrink-0" style={{ width: size, height: size }}>
      {people.slice(0, group ? 2 : 1).map((p, i) => (
        <div
          key={p.address}
          className={`absolute flex items-center justify-center rounded-full bg-avatar font-medium text-text-2 ${group ? "ring-2 ring-bg" : ""}`}
          style={{
            width: inner,
            height: inner,
            fontSize: font,
            left: group && i === 1 ? size - inner : 0,
            top: group && i === 1 ? size - inner : 0,
            zIndex: i,
          }}
        >
          {initials(p.name)}
          <Picture address={p.address} />
        </div>
      ))}
      {accountColor && (
        <span
          aria-hidden
          className={`absolute -right-0.5 -bottom-0.5 z-10 size-3 rounded-full ring-2 ring-bg ${ACCOUNT_BG[accountColor]}`}
        />
      )}
    </div>
  );
}

export function IconButton({
  label,
  children,
  active = false,
  className = "",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  label: string;
  children: ReactNode;
  active?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={active || undefined}
      className={`flex size-7 items-center justify-center rounded-md text-text-2 hover:bg-hover hover:text-text max-[700px]:size-9 ${
        active ? "text-accent-text" : ""
      } ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  if (IS_ANDROID) return null;
  return (
    <kbd className="rounded border border-line px-1 font-sans text-[11px] leading-4 text-text-2">
      {children}
    </kbd>
  );
}

/** A search excerpt from the core with its matches (\u0002 … \u0003) highlighted. */
export function Snippet({ text }: { text: string }) {
  return text
    .replace(/\s+/g, " ")
    .split("\u0002")
    .map((part, i) => {
      if (i === 0) return <Fragment key={i}>{part}</Fragment>;
      const [hit, rest = ""] = part.split("\u0003");
      return (
        <Fragment key={i}>
          <mark className="rounded-[3px] bg-brand/20 text-inherit">{hit}</mark>
          {rest}
        </Fragment>
      );
    });
}

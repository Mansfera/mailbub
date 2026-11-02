import type { TFunction } from "i18next";
import { CoreError } from "./core";
import type { Provider } from "../types";

/** Turns a core error into a translated title plus the server's own text as detail. */
export function describeError(
  t: TFunction,
  provider: Provider,
  e: unknown,
): { title: string; detail?: string } {
  if (!(e instanceof CoreError)) return { title: t("setup.errors.generic"), detail: String(e) };
  let key: string = e.code;
  if (e.code === "imap_auth" && provider !== "imap") key = `imap_auth_${provider}`;
  if (e.code === "smtp_connect" || e.code === "smtp_auth") key = "generic";
  const title = t(`setup.errors.${key}`, { defaultValue: t("setup.errors.generic") });
  return { title, detail: e.message && e.code !== "no_core" ? e.message : undefined };
}

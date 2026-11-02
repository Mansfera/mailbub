import i18n from "../i18n";

const LOCALES: Record<string, string> = { en: "en-GB", uk: "uk-UA" };

function locale(): string {
  return LOCALES[i18n.language] ?? "en-GB";
}

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

function dayDiff(d: Date, now = new Date()): number {
  return Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
}

export function formatTime(iso: string): string {
  return new Intl.DateTimeFormat(locale(), { hour: "2-digit", minute: "2-digit" }).format(
    new Date(iso),
  );
}

/** Chat list: time today, weekday this week, date otherwise. */
export function formatListDate(iso: string): string {
  const d = new Date(iso);
  const diff = dayDiff(d);
  if (diff === 0) return formatTime(iso);
  if (diff === 1) return i18n.t("date.yesterday");
  if (diff < 7) return new Intl.DateTimeFormat(locale(), { weekday: "short" }).format(d);
  return new Intl.DateTimeFormat(locale(), { day: "numeric", month: "short" }).format(d);
}

/** Separator between days inside a conversation. */
export function formatDayHeading(iso: string): string {
  const d = new Date(iso);
  const diff = dayDiff(d);
  if (diff === 0) return i18n.t("date.today");
  if (diff === 1) return i18n.t("date.yesterday");
  const sameYear = d.getFullYear() === new Date().getFullYear();
  return new Intl.DateTimeFormat(locale(), {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: sameYear ? undefined : "numeric",
  }).format(d);
}

export function isSameDay(a: string, b: string): boolean {
  return startOfDay(new Date(a)) === startOfDay(new Date(b));
}

export function formatSize(bytes: number): string {
  const units = ["B", "KB", "MB", "GB"];
  let n = bytes;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n < 10 && i > 0 ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
}

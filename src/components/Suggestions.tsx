import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CalendarPlus,
  Check,
  HelpCircle,
  Loader2,
  MapPin,
  Map as MapIcon,
  Video,
  X,
} from "lucide-react";
import { core, IN_TAURI } from "../lib/core";
import { detect, type EventSuggestion } from "../lib/events";
import { openLink } from "../lib/openLink";
import { usePrefs, type CalendarApp, type MapsApp } from "../store";
import i18n from "../i18n";
import type { Message } from "../types";

const LOCALES: Record<string, string> = { en: "en-GB", uk: "uk-UA" };

function when(e: EventSuggestion): string {
  const opts: Intl.DateTimeFormatOptions = e.allDay
    ? { weekday: "short", day: "numeric", month: "short" }
    : { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" };
  return new Intl.DateTimeFormat(LOCALES[i18n.language] ?? "en-GB", opts).format(e.start);
}

function mapsUrl(app: MapsApp, place: string): string {
  const q = encodeURIComponent(place);
  return app === "google"
    ? `https://www.google.com/maps/search/?api=1&query=${q}`
    : `https://maps.apple.com/?q=${q}`;
}

function gcalStamp(d: Date, allDay: boolean): string {
  const iso = d
    .toISOString()
    .replace(/[-:]/g, "")
    .replace(/\.\d{3}/, "");
  return allDay ? iso.slice(0, 8) : iso;
}

async function addEvent(app: CalendarApp, e: EventSuggestion, notes: string) {
  const end = e.end ?? new Date(e.start.getTime() + (e.allDay ? 86_400_000 : 3_600_000));
  if (app === "google" || !IN_TAURI) {
    const params = new URLSearchParams({
      action: "TEMPLATE",
      text: e.title,
      dates: `${gcalStamp(e.start, e.allDay)}/${gcalStamp(end, e.allDay)}`,
      details: notes,
      ...(e.location ? { location: e.location } : {}),
    });
    openLink(`https://calendar.google.com/calendar/render?${params}`);
    return;
  }
  await core.addToCalendar({
    title: e.title,
    start: Math.floor(e.start.getTime() / 1000),
    end: Math.floor(end.getTime() / 1000),
    allDay: e.allDay,
    location: e.location ?? null,
    notes,
  });
}

/** Free OpenStreetMap preview, loaded only when asked (it contacts OSM). */
function MapPreview({ place }: { place: string }) {
  const { t } = useTranslation();
  const [state, setState] = useState<"loading" | "missing" | { lat: number; lon: number }>(
    "loading",
  );
  useEffect(() => {
    let live = true;
    void core
      .geocode(place)
      .then((p) => live && setState(p ? { lat: p.lat, lon: p.lon } : "missing"))
      .catch(() => live && setState("missing"));
    return () => {
      live = false;
    };
  }, [place]);

  if (state === "loading")
    return (
      <div className="flex h-10 items-center gap-1.5 text-[12px] text-text-2">
        <Loader2 size={13} className="animate-spin" aria-hidden />
      </div>
    );
  if (state === "missing")
    return <p className="text-[12px] text-text-2">{t("suggest.notFound")}</p>;
  const d = 0.006;
  const bbox = [state.lon - d, state.lat - d / 2, state.lon + d, state.lat + d / 2].join(",");
  return (
    <figure className="overflow-hidden rounded-xl border border-line">
      <iframe
        title={place}
        sandbox="allow-scripts allow-same-origin"
        src={`https://www.openstreetmap.org/export/embed.html?bbox=${bbox}&layer=mapnik&marker=${state.lat},${state.lon}`}
        className="block h-48 w-full border-0"
      />
      <figcaption className="bg-surface px-2 py-1 text-[10px] text-text-2">
        {t("suggest.mapCredit")}
      </figcaption>
    </figure>
  );
}

/** Calendar and map actions for dates, times and places found in a message. */
export function Suggestions({ message, align }: { message: Message; align: "start" | "end" }) {
  const { t } = useTranslation();
  const mapsApp = usePrefs((s) => s.maps);
  const calendarApp = usePrefs((s) => s.calendar);
  const found = useMemo(() => detect(message), [message]);
  const join = found.join;
  const [mapFor, setMapFor] = useState<string | null>(null);
  if (!found.events.length && !found.places.length && !found.join && !found.rsvp.length)
    return null;

  const chip =
    "flex h-7 max-w-full items-center gap-1.5 rounded-full border border-line bg-raised px-2.5 text-[12px] hover:bg-hover";
  const appName = (k: string) => t(`settings.${k}`);

  return (
    <div
      className={`mt-1 flex w-[min(560px,92%)] flex-col gap-1.5 ${align === "end" ? "items-end" : "items-start"}`}
    >
      <div className={`flex flex-wrap gap-1.5 ${align === "end" ? "justify-end" : ""}`}>
        {join && (
          <button
            type="button"
            title={join.url}
            onClick={() => openLink(join.url)}
            className="flex h-7 items-center gap-1.5 rounded-full bg-accent px-3 text-[12px] font-medium text-on-accent hover:bg-accent-hover"
          >
            <Video size={14} aria-hidden className="shrink-0" />
            {join.service ? t("suggest.joinWith", { service: join.service }) : t("suggest.join")}
          </button>
        )}
        {found.rsvp.length > 0 && (
          <span
            role="group"
            aria-label={t("suggest.rsvp")}
            className="flex h-7 items-center overflow-hidden rounded-full border border-line bg-raised text-[12px]"
          >
            <span className="px-2.5 text-text-2">{t("suggest.rsvp")}</span>
            {found.rsvp.map((r) => {
              const Icon = r.kind === "yes" ? Check : r.kind === "no" ? X : HelpCircle;
              return (
                <button
                  key={r.kind}
                  type="button"
                  title={t("suggest.rsvpHint")}
                  onClick={() => openLink(r.url)}
                  className="flex h-full items-center gap-1 border-l border-line px-2.5 hover:bg-hover"
                >
                  <Icon
                    size={13}
                    aria-hidden
                    className={
                      r.kind === "yes"
                        ? "text-done"
                        : r.kind === "no"
                          ? "text-account-red"
                          : "text-text-2"
                    }
                  />
                  {t(`suggest.rsvpKind.${r.kind}`)}
                </button>
              );
            })}
          </span>
        )}
        {found.events.map((e) => (
          <button
            key={`${e.start.getTime()}-${e.title}`}
            type="button"
            title={`${t("suggest.addEvent")} · ${appName(calendarApp === "google" ? "googleCalendar" : "appleCalendar")}`}
            onClick={() => void addEvent(calendarApp, e, message.subject)}
            className={chip}
          >
            <CalendarPlus size={14} aria-hidden className="shrink-0 text-accent-text" />
            <span className="font-medium">{t("suggest.addEvent")}</span>
            <span className="truncate text-text-2">{when(e)}</span>
          </button>
        ))}
        {found.places.map((p) => (
          <span key={p} className="flex max-w-full items-center">
            <button
              type="button"
              title={t("suggest.openIn", {
                app: appName(mapsApp === "google" ? "googleMaps" : "appleMaps"),
              })}
              onClick={() => openLink(mapsUrl(mapsApp, p))}
              className={`${chip} rounded-r-none`}
            >
              <MapPin size={14} aria-hidden className="shrink-0 text-account-red" />
              <span className="truncate">{p}</span>
            </button>
            {IN_TAURI && (
              <button
                type="button"
                aria-label={t(mapFor === p ? "suggest.hideMap" : "suggest.showMap")}
                title={t(mapFor === p ? "suggest.hideMap" : "suggest.showMap")}
                aria-pressed={mapFor === p}
                onClick={() => setMapFor(mapFor === p ? null : p)}
                className={`${chip} -ml-px rounded-l-none px-2 ${mapFor === p ? "text-accent-text" : "text-text-2"}`}
              >
                <MapIcon size={14} />
              </button>
            )}
          </span>
        ))}
      </div>
      {mapFor && (
        <div className="w-full max-w-[420px]">
          <MapPreview key={mapFor} place={mapFor} />
        </div>
      )}
    </div>
  );
}

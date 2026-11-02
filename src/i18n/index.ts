import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./en.json";
import uk from "./uk.json";

export const LANGUAGES = [
  { code: "en", label: "English" },
  { code: "uk", label: "Українська" },
] as const;

export type Language = (typeof LANGUAGES)[number]["code"];

export function detectLanguage(): Language {
  return navigator.language.toLowerCase().startsWith("uk") ? "uk" : "en";
}

void i18n.use(initReactI18next).init({
  resources: { en: { translation: en }, uk: { translation: uk } },
  lng: detectLanguage(),
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

export default i18n;

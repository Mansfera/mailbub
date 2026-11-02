/** Login fields: password managers fill them, so the webview must not "correct" the text. */
export const CREDENTIAL = { autoCapitalize: "off", autoCorrect: "off", spellCheck: false } as const;

/** Fields that are not credentials: keeps password managers from offering logins there. */
export const NO_AUTOFILL = {
  autoComplete: "off",
  "data-1p-ignore": "true",
  "data-bwignore": "true",
  "data-lpignore": "true",
} as const;

import { openUrl } from "@tauri-apps/plugin-opener";
import { core, IN_TAURI, IS_ANDROID } from "./core";

const SAFE_LINK = /^(https?:|mailto:|tel:)/i;

/** Opens web, mail and phone links outside the app; ignores javascript:, file: and the like. */
export function openLink(href: string) {
  const url = href.trim();
  if (!SAFE_LINK.test(url)) return;
  if (IN_TAURI) void core.openExternal(url).catch(() => undefined);
  else window.open(url, "_blank", "noopener");
}

/**
 * Opens a sign-in page. On Android that is a Custom Tab over the app: providers refuse web
 * views, and the tab closes by itself when its page sends the user back to the app. The app
 * stays awake meanwhile, or Android would freeze it before the browser comes back to it.
 */
export async function openSignIn(url: string) {
  if (!IS_ANDROID) return openLink(url);
  await core.setSignInActive(true).catch(() => undefined);
  await openUrl(url, "inAppBrowser").catch(() => openLink(url));
}

/** The sign-in finished or was abandoned. */
export function endSignIn() {
  if (IS_ANDROID) void core.setSignInActive(false).catch(() => undefined);
}

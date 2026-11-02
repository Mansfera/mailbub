#!/bin/sh
# Signs a debug binary with the self-signed "Mailbub Code Signing" identity, then runs it.
# Without the identity (other machines, CI) the binary just runs unsigned.
identity="${MAILBUB_SIGNING_IDENTITY:-Mailbub Code Signing}"
if security find-identity -p codesigning 2>/dev/null | grep -q "\"$identity\""; then
  codesign --force --sign "$identity" --identifier com.mansfera.mailbub "$1" 2>/dev/null ||
    echo "sign-and-run: could not sign with \"$identity\"; running unsigned" >&2
fi
exec "$@"

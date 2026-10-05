---
title: Privacy
summary: What stays on your Mac (almost everything) and what doesn't.
---
| Data | Where it lives |
| --- | --- |
| Searches, ranking history | `~/Library/Application Support/Moo` |
| Clipboard history | Memory only, cleared when moo quits |
| Settings and shortcuts | `~/.config/moo/shortcuts.json` |
| API keys, sign-in tokens, plugin secrets | Your login Keychain |
| Apple Intelligence questions | Never leave the Mac |
| Other AI providers | Sent directly to the provider you chose |

The sign-in relay at `moo.moi/callback` forwards your browser back to moo and stores nothing.

The full policy is at [moo.moi/legal/privacy](/legal/privacy).

---
title: Ask AI
summary: Ask a question, get a streamed answer, and let the model act on your Mac.
---
## Asking

Type a question at the root and press [[Tab]]. You can also pick the **Ask AI** row, or open the **Ask AI** command. The answer streams into the launcher.

- Type a follow-up and press [[Return]]. The conversation keeps its context until you leave.
- [[Return]] on an empty field copies the answer.
- [[Esc]] stops a streaming answer, then leaves.
- [[↓]] scrolls the answer.

## It can act

The model has tools, so it can do things instead of guessing at them:

| Ask | It uses |
| --- | --- |
| "find large files from the last 2 days" | File search by size, date, kind, folder or contents |
| "which apps are open?" | Running apps |
| "how much disk space and battery do I have?" | System info |
| "open my documents folder" | Open apps, files and URLs |
| "put the left window on the other display" | Window layouts |
| "turn on dark mode" | System controls |
| "what's 18% of 2,340 in euros?" | The calculator, with today's rates |
| "what does petrichor mean?" | The Mac's dictionary |
| "run my stamp shortcut" | Your shortcuts |

Files, apps and clipboard entries it finds show up as rows under the answer, ready to open.

> **Note** moo never lets the model restart, shut down, log out or empty the Trash. It tells you which command to run instead.

## From the terminal

```sh
moo ask "summarize: $(pbpaste)"
```

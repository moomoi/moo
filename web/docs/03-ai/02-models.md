---
title: Models & providers
summary: On-device by default. Bring Hypery, OpenAI, or a model running on your own machine.
---
## Apple Intelligence (default)

Apple's on-device model is free, works offline, and nothing you ask leaves your Mac. It needs macOS 26 with Apple Intelligence turned on. It's small, so it's best at short answers, rewriting and extracting, and a very long conversation eventually runs out of room. When that happens, press [[Esc]] to start over.

## Other providers

Open **AI Models** to pick a model. Built in:

| Provider | Sign-in |
| --- | --- |
| Hypery | Browser sign-in, or an API key |
| OpenAI | API key |
| Ollama | None, runs locally |
| LM Studio | None, runs locally |

API keys and sign-in tokens are kept in your login Keychain.

## Add your own

Any OpenAI-compatible server works. Add it under `ai.providers` in [shortcuts.json](/docs/shortcuts-json) and pick it by `provider:model`:

```json
{
  "ai": {
    "model": "work:llama-3.1-70b",
    "providers": {
      "work": { "url": "https://llm.example/v1", "keyEnv": "WORK_KEY" }
    }
  }
}
```

## Signing in

Browser sign-in uses OAuth with PKCE. Your browser passes through `moo.moi/callback`, which forwards it straight back to moo on your Mac. The relay keeps nothing, and the code it carries can't be used without a secret that never leaves your Mac.

If a request needs sign-in, credit or a key, moo shows the fix as the selected row, such as **Sign In to Hypery** or **Add API Key**, and asks your question again once that's done.

---
title: Slack
summary: Send, search and open Slack from the launcher. Set up once in a couple of minutes.
---
## Commands

| Command | |
| --- | --- |
| Send Slack Message | `slack` then Space: pick who it's to, type the message, Return |
| Search Slack Messages | Results as you type |
| Open Slack Channel | Jump into a channel or DM |
| Refresh Slack Channels | Reload channels and people |
| Sign In / Sign Out | |

## Setup

Slack needs an app in your workspace. moo walks you through it: run any Slack command and it leads to **Sign In to Slack**, which asks for a Client ID and offers **Create Moo's Slack App** with everything filled in.

1. Create the app in the workspace you want.
2. Under **OAuth & Permissions**, turn on **PKCE**.
3. To use it in other workspaces, turn on public distribution under **Manage Distribution**.
4. Copy the **Client ID** from **Basic Information**, paste it into moo and press [[Return]]. Sign-in opens in your browser.

> **Note** PKCE means no client secret ships with moo. Slack can't turn PKCE off again without contacting support, and it marks the app as a public client, which is right for a desktop app.

Your token is kept in the Keychain. Channels and people are cached for ten minutes.

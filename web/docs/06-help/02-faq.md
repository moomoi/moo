---
title: FAQ
summary: Short answers to common questions.
---
## Is moo free?

Yes.

## Does it replace Spotlight?

It can. Give moo [[⌘ Space]] by turning off Spotlight's shortcut. moo keeps its own compact index of file names, so a search takes a few milliseconds. For words inside files it uses Spotlight's index, so nothing is indexed twice.

## Is it really native?

Yes. moo is one native process built with [Tish](https://github.com/tishlang/tish) on AppKit. There are no webviews, no Electron, no Node and no JavaScript.

## Does it phone home?

No. There are no analytics and no telemetry. moo only goes online for what you ask: AI providers you choose, currency rates, web suggestions, and plugins you use. See [Privacy](/docs/privacy).

## Can I use Raycast extensions?

No. The plugin API borrows Raycast's ideas (commands, arguments, list views), but plugins are written in Tish. See [Your first plugin](/docs/first-plugin).

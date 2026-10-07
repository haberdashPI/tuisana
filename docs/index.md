---
layout: home

hero:
  name: tuisana
  text: Asana, from the keyboard
  tagline: A terminal UI for reviewing and editing Asana projects and tasks, with a shortcut for everything.
  actions:
    - theme: brand
      text: Getting started
      link: /getting-started
    - theme: alt
      text: Configuration
      link: /config/
    - theme: alt
      text: View on GitHub
      link: https://github.com/haberdashPI/tuisana

features:
  - title: Pick projects, then ask a question
    details: Select the projects you care about, then narrow them with filter panels that AND within a tab and OR between tabs. Save the whole question under a name and recall it with a digit.
  - title: The table is writable
    details: Walk a column cursor across the task table and edit the cell under it — assignee, dates, state, projects, parent, custom fields — one row or every selected row at once.
  - title: A Gantt chart that stays in the table
    details: Draw a timeline beside the rows without losing the cursor, the selection, or the scroll position. Colour the bars by assignee, section, state, or any enum custom field.
---

## Is this for you?

tuisana is for people who already know Asana and would rather not reach for the
mouse. It reads your projects over the Asana API and draws them as a table you
move through with `j` and `k`.

Everything is discoverable from inside the app: `?` opens a help overlay for
whatever mode you are in, listing the keys that are actually bound in *your*
config. These pages cover what the app cannot tell you itself — how to install
it, how to authenticate, and what every key in `tuisana.toml` means.

::: warning Early-stage project
This is an early-stage project largely developed with AI assistance. Every file
has been reviewed by a human, but all of the code was written by Codex or
Claude Code.
:::

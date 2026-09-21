# Milestone 14: Edit tasks

[← all milestones](../plan.md)

Goal:

- make it possible to create, edit, and move tasks

Deliverables:

- users can toggle completion of a task
- user can create new tasks that use default values based on the task they are above
  - if the task they are above is a subtask, the new task is a subtask
  - the dates from the task above are used
- user can change the columns of a task; editing follows the same pattern as filter editing
  - dates are entered in YYYY-MM-DD, MM-DD, or keyword, and are translated to YYYY-MM-DD
  - label options can be selected using j/k and deleted using d
  - titles are typed; backspace and cursor motions are possible; basic vim normal mode
    support for editing text (h/l, d, y, c, b, w, $, 0, C, D all work as in vim)
- users can increase the subtask level: the task becomes a subtask of the task directly above it (default keybinding `>`)
- users can decrease the subtask level of a task (default keybinding `<`)
- when subtask adjustment (`<`/`>`) is a bulk action, it adjusts subtask level based on
  the task directly above the first item selected
- when the sort view is "natural", users can move tasks up and down in the list (`shift+j`, `shift+k`)
  and into or out of sections (`m [` / `m ]`, `m {`, `m }`)
- all editing operations can be applied to multiple tasks at once using the selection mechanism from [Milestone 10](10-task-interaction.md)
- all keybindings are configurable

Implementation notes:

- follow the same edit model as the filter editor for consistent field editing UX
- use the selection mechanism from [Milestone 10](10-task-interaction.md) for bulk edits
- prefer optimistic local updates when safe; reconcile state after server confirmation
- model edits as domain mutations rather than UI-specific actions

Acceptance criteria:

- task completion can be toggled
- new tasks can be created with sensible defaults from the adjacent task
- task fields can be edited inline following the same UX as filter editing
- subtask level can be increased and decreased
- tasks can be reordered in natural sort view
- all editing operations apply to the current selection
- all keybindings are configurable
- tests cover field editing, task creation, subtask adjustment, and bulk operations

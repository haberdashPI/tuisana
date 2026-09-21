# Milestone 10: Task interaction ✓

[← all milestones](../plan.md)

Goal:

- make basic task interaction possible: opening tasks in Asana and copying task links

Deliverables:

- the currently highlighted task can be opened in the Asana app (default binding `enter`)
- the user can select tasks for bulk actions
  - like project selection: select all/none, invert; selections depend on the filtered state
  - like project selection: a task not currently visible can remain selected
  - the selection can be cleared of tasks that are not visible
- a "copy to clipboard" action produces a markdown checklist item for each selected task (default binding `y`):
  `- [ ] [task title](task link)`

Implementation notes:

- follow the project selection model for task selection state
- selected tasks not visible in the current filter should remain selected until explicitly cleared
- the clipboard output should be consistent markdown that renders correctly in common tools

Acceptance criteria:

- the highlighted task can be opened in Asana from the task view
- tasks can be selected and deselected; selection state persists through filter changes
- select all, invert, and clear-hidden-selection work on the current filtered task set
- the clipboard action produces a markdown checklist item for each selected task
- tests cover selection state transitions and clipboard output format

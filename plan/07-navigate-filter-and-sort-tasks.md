# Milestone 7: Navigate, filter, and sort tasks

[← all milestones](../plan.md)

Goal:

- navigate the task list with vim-like shortcuts
    - up, down, page-up, page-down
    - move by section
    - move by project
- filter by:
    - field
    - subtask visibility
    - complete/incomplete tasks
    - task owner
    - date range
- sort tasks predictably

Deliverables:

- task navigation actions
- filter model
- sort model
- subtask include/exclude toggle
- stable ordering rules
- keybinding defaults for all commands

Implementation notes:

- filters should be composable where practical
- support filtering by explicit field, text, fuzzy and regex search
- make subtask visibility a first-class filter, not an afterthought
- subtasks always show up under their associated super tasks
- if subtasks are visible, the super task is visible when any of the subtasks match a filter or when the super task matches the filter
- use a stable fallback sort to avoid jumpy rows

Acceptance criteria:

- row navigation works
- filtering by field works
- subtask visibility can be toggled
- sorting is deterministic
- tests cover filter and sort combinations

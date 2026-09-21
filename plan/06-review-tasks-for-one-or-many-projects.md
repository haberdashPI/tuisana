# Milestone 6: Review tasks for one or many projects

[← all milestones](../plan.md)

Goal:

- show tasks in a table for one selected project or a user-defined set of projects

Deliverables:

- a task view that shows up below the project list
- task loading from Asana
- section data loading
- custom field discovery for project-specific fields
- task table model with common columns and project-specific columns
- when visible the task view should take up most of the screen: e.g. just show the first 4-6 lines of the project view.
- task view can be easily toggled as visible / hidden
- user can toggle between "project" mode (which interacts with the project list) and "task" mode (which interacts with the tasks)
    - toggling between these modes only changes visibility of the task list
      when the user switches to the task mode: in this case the task list is
      made visible if it isn't already

Suggested default table fields:

- task name
- section
- assignee
- due date
- start date
- completion state
- project-specific custom fields
- any other useful metadata that is cheap to fetch

Implementation notes:

- keep the table model separate from raw API DTOs
- normalize Asana data before rendering
- support a single-project view and a multi-project aggregated view
- decide explicitly how duplicate tasks across selected projects should be handled

Acceptance criteria:

- tasks can be loaded for one project
- tasks can be loaded across a configured project set
- table columns include key fields and project-specific fields
- tests cover field mapping and multi-project merging

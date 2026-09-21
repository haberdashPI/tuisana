# Milestone 8: Code legibility

[← all milestones](../plan.md)

In progress

Manual review status:

- [X] src/main.rs
- [X] src/app.rs
- [X] src/ui/runtime.rs
- [X] src/config/mode.rs
- [X] src/input/mod.rs
- [X] src/ui/task_table.rs
- [X] src/app/task_review.rs

The remaining files have yet to be cleaned up at all

Goal

- ensure that a someone new to the project can easily understand
  and contribute the code

Deliverables:

- a developer.md doc contains documentation for getting started in
  reading through the code.
- the core data structures are documented with their role in the
  application, especially the app state machine and the runtime input
  flow.
- the names of functions and modules are self-explanatory
- code is often self-documenting: it is clear from the calls
  and functions what is happening.
- There are basic primitives used throughout the project that
  that encaspulate common patterns used throughout the project
- Functions are relatively short: when necessary, large functions have clear documented
  sections.
- The new organization is not needlessly inflexible: it should be easy
  to add or change functionality.
- The new organization is not needlessly abstract: there are a limited
  number of levels of indirection and it is general clear what
  a given piece of code is doing in concrete terms.

Implementation notes:

- This milestone may require a substantial rewrite. Consider whether
  the boundaries and categories of the code actually make sense
  now that we've implemented most of the functionality. The goal
  is to optimize for clarity and legibility over the most
  efficient possible implementation.
- We should keep in mind that we probably want to eventually introduce more error handling;
  that should not substantially reduce legibility once we do it in a future milestone.

Acceptance criteria:
- The behavior and appearance of the application remains unchanged.

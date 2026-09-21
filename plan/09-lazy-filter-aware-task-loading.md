# Milestone 9: Lazy, filter-aware task loading

[← all milestones](../plan.md)

Goal:

- avoid loading a large task set eagerly when only a small subset is needed
- push task retrieval decisions down to the current task filters and selected projects

Deliverables:

- a task query model that can express the active task filters and project scope
- Asana client support for requesting tasks using that query model
- app logic that requests only the data needed for the current task view state
- fallback behavior for cases where the requested query cannot be expressed lazily

Implementation notes:

- keep the query model explicit so the Asana client and fake backend can both implement it
- tailor the request to the active filter set instead of always loading whole project task trees
- preserve the existing eager path as a fallback for unsupported query combinations
- keep lazy loading incremental and cache-aware so switching projects does not discard useful data

Acceptance criteria:

- the app avoids querying the full task set when the active view only needs a narrower subset
- filter-aware task requests work through the existing client abstraction
- unsupported query combinations fall back to the current eager behavior
- tests cover the query model, lazy loading behavior, and fallback path

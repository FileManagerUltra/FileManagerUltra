## ADDED Requirements

### Requirement: Drawing does not grow memory
Drawing frames SHALL NOT make the application's memory grow: the renderer SHALL upload each frame's geometry in one write per buffer into storage it reuses, rather than allocating new buffer storage for every mesh of every frame, so graphics drivers that release such storage late (NVIDIA OpenGL with "Threaded optimization" on Windows) do not accumulate it.

#### Scenario: Window visible on Windows with an NVIDIA GPU
- **WHEN** the application is visible for ten minutes with video playing or the pointer moving over it, with the driver's default settings
- **THEN** its commit size stays flat apart from what open files and caches need, as it does while minimized

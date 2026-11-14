## ADDED Requirements

### Requirement: Drawing does not grow memory
Drawing frames SHALL NOT make the application's memory grow. On Windows the window SHALL be drawn with Direct3D 12 by default rather than OpenGL, because OpenGL drivers there (NVIDIA with "Threaded optimization") keep memory for every drawn frame.

#### Scenario: Window visible on Windows with an NVIDIA GPU
- **WHEN** the application is visible for ten minutes with video playing or the pointer moving over it, with the driver's default settings
- **THEN** its commit size stays flat apart from what open files and caches need, as it does while minimized

### Requirement: Renderer choice on Windows
On Windows, Preferences › Advanced › Troubleshooting SHALL offer *Renderer*: Direct3D (default) or OpenGL, applied at the next start, and SHALL show which renderer and GPU are in use. If Direct3D cannot start, the application SHALL open with OpenGL instead and say so in a notice. Other platforms draw with OpenGL and do not show the choice.

#### Scenario: Choosing OpenGL
- **WHEN** the user selects OpenGL and restarts the application
- **THEN** the window is drawn with OpenGL and the row shows "In use: OpenGL"

#### Scenario: Direct3D unavailable
- **WHEN** Direct3D 12 cannot start on the machine, or only offers a software adapter
- **THEN** the window opens with OpenGL and a notice explains that Direct3D could not start

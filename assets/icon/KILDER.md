# App icon

`desktop-hygge.png` is the app icon of the Microsoft Store version (Desktop Hygge): a mug of hot cocoa in a red
knitted sleeve, on a transparent background, 576x576.

- Made on 2026-11-22 with ChatGPT's image model by the project owner, from a prompt asking for a cosy Windows 11
  style app icon with no text or logos (OpenAI's terms give the user the rights to the output, including
  commercial use).
- Picked from a sheet of four mugs and cleaned by a script: faint pixels and small stray specks around the steam
  removed, cropped to the mug and centred on a square with a 3% margin.

`tools\AppIcon.ps1` (`Get-AppIconBitmap`) scales it to every size: the exe's `.ico` (`build.ps1`), the package
logos (`store\make-msix.ps1`) and the Store listing logos. The tray icon by the clock is still drawn in code.
For a sharper 1080x1080 Store logo, a larger version of the same image would be needed.

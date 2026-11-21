# Microsoft Store

The Windows version can be sold through the Microsoft Store as an MSIX package. The Store signs the package (so
no SmartScreen warning), installs and updates it, and handles payment and VAT.

## What differs in the package

- `Program.IsPackaged` (`GetCurrentPackageFullName`) tells the app it runs from a package.
- No self-update (`Updater.SelfUpdates` is false): the Store installs updates, and the install folder is
  read-only. The menu has no update items; the version line stays.
- Autostart is the package's startup task (`windows.startupTask` in `store/AppxManifest.xml`, off by default),
  because the Run key is virtualized in a package. The menu item "Start med Windows" opens Windows' Startup apps
  page (`ms-settings:startupapps`), where the user switches it on.
- Settings and logs: the package redirects `%APPDATA%\JuleHygge` to its own folder, so the Store version and a
  downloaded exe do not share settings.

## Build

- `store\make-msix.ps1 -Version x.y.z -IdentityName ... -Publisher ... -PublisherDisplayName ...` builds the exe,
  lays out the package in `obj\store\layout` (exe, manifest, logos drawn by `tools\AppIcon.ps1`) and packs it with
  the Windows SDK's `makeappx.exe`. The development PC has no SDK, so locally it stops after the layout.
- The Store version is developed on the `microsoft-store` branch, apart from `main` (the GitHub releases).
- The **Store package** workflow (`.github/workflows/store.yml`, runs on every push to `microsoft-store`) packs it on GitHub's Windows runner
  and keeps `JuleHygge-x.y.z.msix` as a workflow artifact. The identity comes from the repository variables
  `STORE_IDENTITY_NAME`, `STORE_PUBLISHER` and `STORE_PUBLISHER_NAME`, copied from Partner Center (the app's
  Product identity page), and the app name from `STORE_DISPLAY_NAME`, which must be a name reserved there (the
  working name is "Desktop Hygge", not final).
- Each Store update needs a higher version than the last one submitted.

## Test locally

Turn on Developer Mode (Settings, System, For developers), then `store\make-msix.ps1 -Register` installs the
layout for the current user with a test identity. Start JuleHygge from the Start menu. Remove it with
`Get-AppxPackage JuleHygge.Test | Remove-AppxPackage`. Without Developer Mode, registration fails with
0x80073CFF (the manifest itself was accepted on 2026-11-21).

## Partner Center checklist

1. Create a developer account at partner.microsoft.com (individual or company).
2. Reserve the name "JuleHygge" (Apps and games, New product, MSIX or PWA app).
3. Copy the three identity values from Product management, Product identity into the repository variables.
4. Run the Store package workflow and upload the `.msix` under the submission's Packages.
5. Fill in pricing, markets, age ratings, the Store listing (description, screenshots in each language) and a
   privacy policy URL. Under Properties, declare that the app uses `runFullTrust` (a desktop app that draws an
   overlay over all screens and reads window positions for the snow on windows).

Not done yet: a free trial (needs the Store licensing API), and Store listing texts and screenshots.

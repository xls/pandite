# Mac App Store submission

This procedure puts Pandite in the Mac App Store. The Mac App Store accepts
only apps that run in the App Sandbox. Pandite is one program for every
distribution. It finds out at start if it runs in the App Sandbox. The
store build differs from the Developer ID build only in its signature, its
entitlements and its package.

## How Pandite works in the App Sandbox

- Pandite reads only the files that the user opens and the folders that
  the user allows. Its settings are in its container:
  `~/Library/Containers/net.darklabs.pandite/Data/Library/Application Support/pandite/`.
- When the user opens an image and Pandite cannot read its folder, Pandite
  shows that image alone. A notice at the bottom of the window reads "Allow
  Pandite to show the other images in this folder".
- The button "Allow..." opens the system folder panel at the folder of the
  image. The user selects that folder or a folder above it. Pandite then
  shows the gallery and the other images of the folder.
- The close mark or Esc closes the notice. For that folder, the notice does
  not come back until Pandite starts again.
- Pandite keeps each allowed folder as a security-scoped bookmark in the
  file `grants` in the settings folder. It also keeps the folders of Move
  to and Copy to. At the next start, Pandite has access to these folders
  again. An allowed folder includes its subfolders.
- Outside the App Sandbox, Pandite works as before. It shows no notice and
  makes no `grants` file.

## The identifiers

| Item | Value |
|---|---|
| Bundle identifier (App ID) | `net.darklabs.pandite`, from `packaging/macos/Info.plist` |
| Entitlements | `packaging/macos/pandite.entitlements` |
| Store package | `dist/pandite-<version>-macos-appstore.pkg` |
| Sandbox test disk image | `dist/pandite-<version>-universal-macos-sandbox-test.dmg` |

The entitlements file turns on the App Sandbox
(`com.apple.security.app-sandbox`), read and write access to the files and
folders that the user selects
(`com.apple.security.files.user-selected.read-write`) and app-scoped
bookmarks (`com.apple.security.files.bookmarks.app-scope`). It has no
network entitlement. For the store package, the build script adds the
application identifier and the team identifier of the provisioning
profile.

## 1. Make the certificates and the profile

You do these steps one time, in the Apple developer account.

1. Sign in to https://developer.apple.com/account.
2. Open Certificates, Identifiers & Profiles.
3. Open Identifiers. Make sure that the App ID `net.darklabs.pandite`
   exists. If it does not exist, select +, then App IDs, then App. Type the
   description `Pandite` and the explicit bundle ID `net.darklabs.pandite`.
4. On your Mac, open Keychain Access. Select Keychain Access, Certificate
   Assistant, Request a Certificate From a Certificate Authority.
5. Type your email address and your name. Select "Saved to disk". Save the
   file `CertificateSigningRequest.certSigningRequest`.
6. In the developer account, open Certificates and select +.
7. Select "Apple Distribution". Upload the request file. Download the
   certificate.
8. Double-click the downloaded certificate. Keychain Access adds it.
9. Select + again. Select "Mac Installer Distribution". Upload the same
   request file. Download the certificate.
10. Double-click the downloaded certificate.
11. In Terminal, list the identities:

    ```sh
    security find-identity -v
    ```

12. Find the two names. The application identity starts with
    `Apple Distribution:`. The installer identity starts with
    `3rd Party Mac Developer Installer:`. Each name ends with your team
    identifier in parentheses. If the names start differently, confirm the
    certificate types in Apple's documentation.
13. In the developer account, open Profiles and select +.
14. Under Distribution, select "Mac App Store Connect". If this name is not
    there, select the Mac App Store distribution type. Confirm the name in
    Apple's documentation.
15. Select the App ID `net.darklabs.pandite`. Select the Apple Distribution
    certificate. Type the name `Pandite Mac App Store`.
16. Download the profile. The file name ends with `.provisionprofile`.

## 2. Make the app record

1. Sign in to https://appstoreconnect.apple.com.
2. Open Apps and select +, then New App.
3. Select the platform macOS.
4. Type the name `Pandite`. Select the primary language.
5. Select the bundle ID `net.darklabs.pandite`.
6. Type a SKU, for example `pandite`.
7. Select Create.
8. Complete the app information with the facts below:
   - Category: Photography. `Info.plist` gives the same category.
   - Price: free.
   - App Privacy: Pandite collects no data. It has no telemetry and no
     advertising, and it does not use the network.
   - Age rating: Pandite shows the image files that the user opens. It has
     no user content from other people, no purchases and no web access.

## 3. Build the binaries

You need a Mac with the tools in "macOS" in [building.md](building.md).

1. Open Terminal in the repository folder.
2. Build both architectures:

   ```sh
   MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release -p pandite-app --target aarch64-apple-darwin --target x86_64-apple-darwin
   ```

Each upload to App Store Connect needs a higher version than the upload
before it. Before you build again, raise the version in `Cargo.toml`.

## 4. Test the App Sandbox

The sandbox test disk image holds the normal app with the App Sandbox
entitlements. It needs no provisioning profile. Test with it before you
make the store package.

1. To sign with your Developer ID, set the identity. Without it, the
   script signs ad hoc, and the app runs only on your Mac.

   ```sh
   export PANDITE_MACOS_SIGN_IDENTITY="Developer ID Application: <name> (<team id>)"
   ```

2. Build the disk image. Replace `0.2.5` with the version:

   ```sh
   PANDITE_MACOS_SANDBOX_TEST=1 bash packaging/macos/build-app.sh 0.2.5 dist target
   ```

3. Open `dist/pandite-0.2.5-universal-macos-sandbox-test.dmg`.
4. Drag Pandite to Applications. This replaces an installed Pandite.
5. Make sure that the app has the entitlements:

   ```sh
   codesign -d --entitlements - /Applications/Pandite.app
   ```

6. Start Pandite. In Activity Monitor, select View, Columns, Sandbox. Make
   sure that the Sandbox column shows Yes for Pandite.
7. Do the tests in the list below, in this order.
8. To test again without allowed folders, quit Pandite. Then remove the
   file `grants`:

   ```sh
   rm ~/Library/Containers/net.darklabs.pandite/Data/Library/Application\ Support/pandite/grants
   ```

9. When you are done, install the Developer ID disk image again. The
   Developer ID build keeps its settings in
   `~/Library/Application Support/pandite/`. It does not use the
   settings in the container.

### Tests

1. Double-click an image in Finder. Pandite shows the image. The gallery
   shows one image. The notice "Allow Pandite to show the other images in
   this folder" shows.
2. Select Allow.... The folder panel opens at the folder of the image.
3. Select a folder that does not hold the image. Pandite shows "Choose the
   folder of the image, or a folder that holds it". The notice stays.
4. Select Allow... again. Select the folder of the image. The gallery shows
   all images of the folder. The notice goes. The Right arrow shows the
   next image.
5. Quit Pandite. Double-click an image in the same folder. The gallery
   shows all images at once, without the notice.
6. Double-click an image in a subfolder of that folder. The gallery shows
   all images without the notice.
7. Double-click an image in another folder. Press Esc. The notice goes and
   Pandite stays open. Open another image of that folder with Command+O. The
   notice does not come back. Press Esc again. Pandite closes.
8. Press M on an image. Select a folder in the panel. The image moves.
   Press M again. The panel opens in that folder. Quit and start Pandite.
   Press M again. The panel opens in that folder, and the move works.
9. Press Shift+M and copy an image to a folder. Make sure that the copy is
   there.
10. Press Command+Shift+S. Save the image in a folder that you did not allow,
    for example Desktop. Make sure that the file is there. Save again over
    the same file.
11. Press the forward delete key (Fn+Delete on a laptop) on an image of an
    allowed folder. The image goes to the Trash. Do the same on an image of
    a folder that you did not allow. Write down what happens.
12. Press Command+B. Convert the images of an allowed folder into a folder
    that you select in the panel.
13. Drop an image from Finder on the window. Drop a folder on the window.
    Write down if the gallery shows the folder.
14. Copy an image with Command+C. Paste it in Preview with File, New from
    Clipboard.
15. Move an allowed folder to another place in Finder. Start Pandite and
    open an image in the moved folder. The gallery shows all images.

## 5. Build the store package

1. Set the three values. Replace the names with your identities and the
   path of your profile:

   ```sh
   export PANDITE_MACOS_STORE=1
   export PANDITE_MACOS_STORE_APP_IDENTITY="Apple Distribution: <name> (<team id>)"
   export PANDITE_MACOS_STORE_INSTALLER_IDENTITY="3rd Party Mac Developer Installer: <name> (<team id>)"
   export PANDITE_MACOS_PROVISIONING_PROFILE="$HOME/Downloads/Pandite_Mac_App_Store.provisionprofile"
   ```

2. Build the package. Replace `0.2.5` with the version:

   ```sh
   bash packaging/macos/build-app.sh 0.2.5 dist target
   ```

3. Find `dist/pandite-0.2.5-macos-appstore.pkg`.

The script puts the profile into the app as
`Contents/embedded.provisionprofile`. It signs the app with the
entitlements and the identifiers of the profile, and it signs the package
with the installer identity. The script stops when the profile is for
another bundle identifier. The store package is not notarized. Apple signs
the apps that the Mac App Store accepts.

The Release workflow on GitHub does not make the store package. Build the
store package on your Mac.

## 6. Upload the package

Use one of the two procedures.

### Procedure A: Transporter

1. Install Transporter from the Mac App Store. Transporter is an app from
   Apple.
2. Start Transporter. Sign in with the Apple ID of the developer account.
3. Drag `dist/pandite-0.2.5-macos-appstore.pkg` into the window.
4. Select Deliver.
5. When Transporter shows an error, read it, correct the problem and build
   the package again.

### Procedure B: command line

1. Make an app-specific password for your Apple ID at
   https://account.apple.com.
2. Upload the package:

   ```sh
   xcrun altool --upload-app --type macos --file dist/pandite-0.2.5-macos-appstore.pkg --username "<apple id>" --password "<app-specific password>"
   ```

Confirm this command in Apple's documentation before you use it. Apple
changes the upload tools from time to time. Do not use `xcrun notarytool`
for the store package. It is only for the Developer ID build.

## 7. Submit the app for review

1. In App Store Connect, open the app. Wait until the build shows under
   the version. This can take some minutes.
2. Optional: test the build with TestFlight before the review.
3. Select the build for the version.
4. Add at least one screenshot of the app.
5. In "Notes for review", use the text below.
6. Select "Add for Review", then submit.

> Pandite is an offline image viewer. No account, sign-in or network is
> needed. To test: start Pandite, press Command+O and open a JPEG or PNG
> image. Pandite shows a notice that asks for access to the folder of the
> image. Select "Allow..." and select the folder of the image. Then press
> the Right arrow key to see the next image in the folder. Press F1 to see
> all keyboard shortcuts.

## 8. After the review

1. App Store Connect shows the result of the review. If Apple rejects the
   app, read the report, correct the problem and upload a new build.
2. When the app is in the Mac App Store, add its link to `README.md`.

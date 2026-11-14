# Microsoft Store submission

This procedure puts pandite in the Microsoft Store. pandite is a Win32
desktop program in an MSIX package. The build makes unsigned packages,
because the Store signs every package that it accepts.

## The three placeholder values

The manifest `packaging/windows/AppxManifest.xml` contains three
placeholder values. Partner Center gives the real values when you reserve
the app name. Do not write these values into the manifest. Give them to
the build script.

| Manifest value | Placeholder in the manifest | Name in Partner Center | Environment variable or repository variable | Script argument |
|---|---|---|---|---|
| `Identity Name` | `REPLACE-ME.pandite` | Package/Identity/Name | `PANDITE_MSIX_IDENTITY_NAME` | `--identity-name` |
| `Identity Publisher` | `CN=REPLACE-ME-PUBLISHER-ID` | Package/Identity/Publisher | `PANDITE_MSIX_PUBLISHER` | `--publisher` |
| `PublisherDisplayName` | `REPLACE-ME publisher display name` | Package/Properties/PublisherDisplayName | `PANDITE_MSIX_PUBLISHER_DISPLAY_NAME` | `--publisher-display-name` |

The script `packaging/windows/build-msix.mjs` also sets the package
version from the app version (`0.1.0` becomes `0.1.0.0`) and the processor
architecture (`x64` or `arm64`).

## 1. Reserve the name

1. Sign in to Partner Center at https://partner.microsoft.com/dashboard
   with your developer account.
2. Go to Apps and games. Select New product, then MSIX or PWA app.
3. Type `pandite` and select Check availability.
4. If the name is available, select Reserve product name.
5. If the name is not available, reserve a different name. Then put the
   reserved name in `packaging/windows/AppxManifest.xml` in three places:
   the `<DisplayName>` element, the `DisplayName` attribute of
   `uap:VisualElements` and the `ShortName` attribute of `uap:DefaultTile`.
   The Store rejects a package whose display name is not a reserved name.

## 2. Copy the identity values

1. In Partner Center, open the app. Go to Product management, then Product
   identity.
2. Copy these three values:
   - Package/Identity/Name
   - Package/Identity/Publisher
   - Package/Properties/PublisherDisplayName

## 3. Build the package

Use one of the two procedures.

### Procedure A: GitHub release

1. In the GitHub repository, go to Settings, Secrets and variables, Actions.
2. Select the Variables tab.
3. Make three repository variables with the values from step 2:
   - `PANDITE_MSIX_IDENTITY_NAME`
   - `PANDITE_MSIX_PUBLISHER`
   - `PANDITE_MSIX_PUBLISHER_DISPLAY_NAME`
4. Push a release tag (see "Release" in [building.md](building.md)).
5. When the Release workflow is complete, open the GitHub release.
6. Download `pandite-<version>-windows.msixbundle`. This bundle contains
   the x64 and the ARM64 package.

### Procedure B: local build

You need Node.js, Rust and the Windows SDK. See [building.md](building.md).

1. Open PowerShell in the repository folder.
2. Set the three values, and link the C runtime statically:

   ```powershell
   $env:PANDITE_MSIX_IDENTITY_NAME = "<Package/Identity/Name>"
   $env:PANDITE_MSIX_PUBLISHER = "<Package/Identity/Publisher>"
   $env:PANDITE_MSIX_PUBLISHER_DISPLAY_NAME = "<Package/Properties/PublisherDisplayName>"
   $env:RUSTFLAGS = "-C target-feature=+crt-static"
   ```

3. Build the x64 package:

   ```powershell
   node packaging/windows/build-msix.mjs --build --require-identity
   ```

4. Build the ARM64 package:

   ```powershell
   rustup target add aarch64-pc-windows-msvc
   node packaging/windows/build-msix.mjs --build --require-identity --target aarch64-pc-windows-msvc
   ```

5. Bundle the two packages. Replace `0.1.0` with the version:

   ```powershell
   node packaging/windows/build-msix.mjs --bundle dist/pandite-0.1.0-x86_64-windows.msix dist/pandite-0.1.0-aarch64-windows.msix
   ```

6. Find `dist\pandite-0.1.0-windows.msixbundle`.

With `--require-identity`, the script stops when a value is missing or when
the program needs the Visual C++ Redistributable. Without it, the script
keeps the placeholders and prints a note. The Store rejects a package that
contains the placeholders.

## 4. Test the package on your computer (optional)

An unsigned package does not install. To test the package before you
submit it, sign a copy with a test certificate. The certificate subject
must be equal to the Publisher value.

1. Open Windows PowerShell as administrator in the repository folder.
2. Make a test certificate. Replace `<Publisher>` with the Publisher value,
   for example `CN=12345678-ABCD-1234-ABCD-1234567890AB`:

   ```powershell
   $cert = New-SelfSignedCertificate -Type Custom -Subject "<Publisher>" -KeyUsage DigitalSignature -FriendlyName "pandite test signing" -CertStoreLocation "Cert:\CurrentUser\My" -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
   $password = ConvertTo-SecureString -String "<password>" -Force -AsPlainText
   Export-PfxCertificate -Cert $cert -FilePath pandite-test.pfx -Password $password
   Export-Certificate -Cert $cert -FilePath pandite-test.cer
   Import-Certificate -FilePath pandite-test.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople
   ```

3. Sign a copy of the package:

   ```powershell
   Copy-Item dist\pandite-0.1.0-x86_64-windows.msix dist\pandite-test.msix
   $env:PANDITE_SIGN_PASSWORD = "<password>"
   node packaging/windows/sign-msix.mjs --pfx pandite-test.pfx dist\pandite-test.msix
   ```

4. Double-click `dist\pandite-test.msix` and select Install.
5. Start pandite from the Start menu. Open an image. Right-click an image
   in File Explorer and select Open with, then pandite.
6. Optional: run the Windows App Certification Kit on the signed package:

   ```powershell
   & "C:\Program Files (x86)\Windows Kits\10\App Certification Kit\appcert.exe" test -appxpackagepath dist\pandite-test.msix -reportoutputpath $env:TEMP\pandite-wack.xml
   ```

7. Remove the test app in Settings, Apps, Installed apps.
8. Do not submit the test copy. Submit the unsigned package or bundle.

### Signed packages in GitHub releases (optional)

When the repository has the two secrets below, the Release workflow also
adds a signed `pandite-<version>-<arch>-windows-sideload.msix` for each
architecture. Users who trust your certificate can install these files
without the Store.

| Secret | Value |
|---|---|
| `PANDITE_SIGNING_CERT_BASE64` | The `.pfx` file in base64 |
| `PANDITE_SIGNING_CERT_PASSWORD` | The `.pfx` password |

1. Write the base64 text of the `.pfx` file to a text file:

   ```powershell
   [Convert]::ToBase64String([IO.File]::ReadAllBytes((Resolve-Path pandite-test.pfx))) | Set-Content pandite-test.pfx.txt
   ```

2. In the GitHub repository, go to Settings, Secrets and variables, Actions.
3. On the Secrets tab, make the two secrets. Copy the text of
   `pandite-test.pfx.txt` into `PANDITE_SIGNING_CERT_BASE64`.
4. Delete `pandite-test.pfx.txt`.

The Store packages stay unsigned. The workflow does not need these secrets.

## 5. Make the submission

1. In Partner Center, open the app and select Start submission.
2. Complete each section with the information below.
3. Select Submit to the Store.

### Pricing and availability

1. Select the markets.
2. Select the price. pandite is free.
3. Select the visibility.

### Properties

1. Category: Photo and video.
2. Privacy policy: the Store asks for a privacy policy when an app
   accesses, collects or sends personal information. pandite does none of
   these. If you give a privacy policy, use the privacy statement below.
3. System requirements: select DirectX 12 as recommended hardware. On
   Windows, pandite draws with Direct3D 12.

### Age ratings

1. Start the IARC questionnaire.
2. Select the category "All other app types".
3. Answer the questions with the facts below:
   - pandite shows the image files that the user opens.
   - Users cannot communicate or share content with other users.
   - pandite has no purchases and no advertising.
   - pandite does not share the location of the user.
   - pandite gives no access to the internet.

### Packages

1. Upload `pandite-<version>-windows.msixbundle`.
2. Make sure that Partner Center shows the x64 and the ARM64 package and the
   Windows Desktop device family.
3. Each new submission needs a higher version than the previous one. Before
   you build, raise the version in `Cargo.toml`. The fourth number of the
   package version is always 0.

### Store listing checklist

- [ ] Description
- [ ] Short description
- [ ] What's new in this version (for updates)
- [ ] Product features (for example: opens 36 image file extensions,
      works offline, never freezes on cloud-drive files, bulk convert and
      resize)
- [ ] At least one screenshot of the desktop app (PNG)
- [ ] Store logos (optional)
- [ ] Search terms (for example: image viewer, photo viewer, RAW, HEIC,
      WebP, AVIF, JPEG XL)
- [ ] Copyright and trademark information
- [ ] Support contact information or website
- [ ] Privacy policy URL (optional, see Properties)

### Restricted capability: runFullTrust

Partner Center asks why the app uses `runFullTrust`. Use this text:

> pandite is a desktop image viewer written in Rust. It is a native Win32
> program in an MSIX package, so it needs runFullTrust to run as a desktop
> app. It opens the image that the user selects or double-clicks. It reads
> the other images in the same folder to show the next image and the
> thumbnail strip. It writes image files only where the user saves them.
> pandite declares no network capability and does not use the network.

### Notes for certification

Use this text in Submission options, Notes for certification:

> pandite is an offline image viewer. No account, sign-in or network is
> needed. To test: start pandite, press Ctrl+O and open a JPEG or PNG
> image. Press the Right arrow key to show the next image in the folder.
> Press F1 to see all keyboard shortcuts. HEIC files open only when the
> HEIF Image Extensions and HEVC Video Extensions from the Microsoft Store
> are installed.

### Privacy statement

Use this text where a privacy statement is necessary:

> pandite does not collect, store or send personal information. It has no
> telemetry and no advertising, and it does not use the network. pandite
> keeps its settings (window size and position, and similar preferences) in
> a settings file on your computer.

## 6. After certification

1. Partner Center shows the certification result. If the Store rejects the
   submission, read the report, correct the problem and submit again.
2. When the app is in the Store, copy the Store link from Product
   management, Product identity. Add the link to `README.md`.

# RPM spec of pandite. packaging/linux/build-rpm.sh defines
#   pandite_version  the release version, with "~" for a pre-release
#   pandite_source   the unpacked Linux release tarball
# and runs rpmbuild. The package holds the files of the tarball under /usr.
# rpmbuild derives Requires from the program (libc, libgcc_s). The program
# loads the Vulkan or EGL driver and the X11 or Wayland libraries at run
# time, so they are weak dependencies (Recommends) by soname.

%{!?pandite_version:%{error:define pandite_version (packaging/linux/build-rpm.sh does this)}}
%{!?pandite_source:%{error:define pandite_source (packaging/linux/build-rpm.sh does this)}}

%global debug_package %{nil}
%global _build_id_links none

Name:           pandite
Version:        %{pandite_version}
Release:        1
Summary:        Fast offline image viewer
License:        MIT
URL:            https://github.com/xls/pandite
BuildArch:      x86_64
Recommends:     libvulkan.so.1()(64bit)
Recommends:     libEGL.so.1()(64bit)
Recommends:     libxkbcommon.so.0()(64bit)
Recommends:     libxkbcommon-x11.so.0()(64bit)
Recommends:     libwayland-client.so.0()(64bit)
Recommends:     libwayland-cursor.so.0()(64bit)
Recommends:     libwayland-egl.so.1()(64bit)
Recommends:     libX11.so.6()(64bit)
Recommends:     libX11-xcb.so.1()(64bit)
Recommends:     libXcursor.so.1()(64bit)
Recommends:     libXi.so.6()(64bit)
Recommends:     libXrandr.so.2()(64bit)

%description
Pandite is a GPU image viewer written in Rust with wgpu. It opens JPEG,
PNG, GIF, WebP, AVIF, JPEG XL, TIFF, OpenEXR, SVG, Photoshop and camera
RAW files. Reads and decodes run on worker threads, so a slow file never
freezes the window. It has no account, no telemetry and no network code.

%prep

%build

%install
src="%{pandite_source}"
install -D -m 0755 "$src/pandite" "%{buildroot}%{_bindir}/pandite"
install -D -m 0644 "$src/share/applications/pandite.desktop" "%{buildroot}%{_datadir}/applications/pandite.desktop"
for dir in "$src"/share/icons/hicolor/*/; do
  size=$(basename "$dir")
  install -D -m 0644 "$dir/apps/pandite.png" "%{buildroot}%{_datadir}/icons/hicolor/$size/apps/pandite.png"
done
install -D -m 0644 "$src/LICENSE" "%{buildroot}%{_datadir}/licenses/pandite/LICENSE"
install -D -m 0644 "$src/README.md" "%{buildroot}%{_datadir}/doc/pandite/README.md"
install -D -m 0644 "$src/THIRD-PARTY-NOTICES.md" "%{buildroot}%{_datadir}/doc/pandite/THIRD-PARTY-NOTICES.md"

%post
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q %{_datadir}/applications || :
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t -f %{_datadir}/icons/hicolor || :
fi

%postun
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q %{_datadir}/applications || :
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t -f %{_datadir}/icons/hicolor || :
fi

%files
%{_bindir}/pandite
%{_datadir}/applications/pandite.desktop
%{_datadir}/icons/hicolor/*/apps/pandite.png
%dir %{_datadir}/licenses/pandite
%license %{_datadir}/licenses/pandite/LICENSE
%dir %{_datadir}/doc/pandite
%doc %{_datadir}/doc/pandite/README.md
%doc %{_datadir}/doc/pandite/THIRD-PARTY-NOTICES.md

# Family Connect for Linux — the .rpm for Fedora 44 and later (docs/packaging.md §12).
#
# Built by scripts/package-rpm.sh from a staged tree of an already compiled, stripped binary and
# its files (fc_stage), so there is no %%build; fc_version comes from Cargo.toml. The files are the
# .deb's (§1) without what only Debian and Ubuntu use: the AppArmor profile (Fedora confines with
# SELinux and lets WebKit's sandbox make its user namespace) and lintian's data. The desktop and
# icon caches are refreshed by the file triggers of desktop-file-utils and hicolor-icon-theme, so
# there are no scriptlets.

%global debug_package %{nil}

Name:           family-connect
Version:        %{fc_version}
Release:        1%{?dist}
Summary:        Family chat and calls on the family's own server
# Family Connect and every crate linked into it: MIT, but foldhash (rusqlite's statement-cache
# hash): Zlib. The copyright file names each crate.
License:        MIT AND Zlib
URL:            https://nettrash.me/linux/familyconnect/support.html

# The libraries the binary links are found by rpm's generators (by soname). What it loads at run
# time is required here: GStreamer's elements for calls and media by the capability each Fedora
# plugin package provides — WebRTC, ICE, Opus, VP8, MP4, AAC, PipeWire capture
# (docs/linux-client-2026-10-28.md) and the sink that draws video in the window
# (gtk4paintablesink, gstreamer1-plugin-gtk4) — TLS for GIO, and the two libraries the client
# will link (the Secret Service and SQLite) and does not yet.
Requires:       hicolor-icon-theme
Requires:       glib-networking%{?_isa}
Requires:       libsecret-1.so.0()(64bit)
Requires:       libsqlite3.so.0()(64bit)
Requires:       gstreamer1(element-webrtcbin)()(64bit)
Requires:       gstreamer1(element-nicesrc)()(64bit)
Requires:       gstreamer1(element-opusenc)()(64bit)
Requires:       gstreamer1(element-vp8enc)()(64bit)
Requires:       gstreamer1(element-mp4mux)()(64bit)
Requires:       gstreamer1(element-avenc_aac)()(64bit)
Requires:       gstreamer1(element-pipewiresrc)()(64bit)
Requires:       gstreamer1(element-gtk4paintablesink)()(64bit)
# H.264: Fedora has no x264enc; openh264enc's library comes from Cisco's repository, which
# Fedora Workstation enables. A weak dependency, so the package still installs where it is off.
Recommends:     gstreamer1(element-openh264enc)()(64bit)
# As the .deb: the Secret Service that keeps the session token, and GNOME's portals (the camera,
# notifications, the global shortcut). Fedora Workstation has both.
Recommends:     gnome-keyring
Recommends:     xdg-desktop-portal-gnome

%description
Family Connect is a chat for one household, or several, on a server the
family runs itself. The messages live in that server's database, and the apps
for iPhone, iPad, Mac, Android, Windows, the web and Linux talk to it for
every chat.

Every family has one chat for all its members, and any two members can talk
one to one: messages, photos, videos, voice notes, files, locations and polls,
one-to-one voice and video calls, a family board and birthdays.

%install
cp -a %{fc_stage}/. %{buildroot}/

%files
%dir %{_datadir}/licenses/%{name}
%license %{_datadir}/licenses/%{name}/copyright
%dir %{_docdir}/%{name}
%doc %{_docdir}/%{name}/CHANGELOG
%{_bindir}/family-connect
%{_datadir}/applications/me.nettrash.FamilyConnect.desktop
%{_datadir}/%{name}/
%{_datadir}/icons/hicolor/*/apps/me.nettrash.FamilyConnect.png
%{_datadir}/icons/hicolor/symbolic/apps/me.nettrash.FamilyConnect-symbolic.svg
%{_datadir}/dbus-1/services/me.nettrash.FamilyConnect.service
%{_datadir}/metainfo/me.nettrash.FamilyConnect.metainfo.xml
%{_mandir}/man1/family-connect.1*

%changelog
* Wed Oct 28 2026 nettrash <nettrash@nettrash.me> - 1.3.0-1
- First release for Linux: Family Connect for Fedora 44 and later, x86_64 and
  aarch64. The family's release history is CHANGELOG beside this package's
  documentation.

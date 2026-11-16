Name:           tarn
Version:        0.1.1
Release:        1%{?dist}
Summary:        Private web browser on the system WebKitGTK 6.0
License:        GPL-3.0-or-later
URL:            https://github.com/Quavon-dev/tarn-browser
Source0:        %{url}/archive/refs/tags/v%{version}/tarn-%{version}.tar.gz

BuildRequires:  gcc pkgconf-pkg-config gtk4-devel webkitgtk6.0-devel
BuildRequires:  gstreamer1-devel gstreamer1-plugins-base-devel
Requires:       webkitgtk6.0
Requires:       gtk4
Requires:       hicolor-icon-theme

%global debug_package %{nil}

%description
Tarn with Shield levels, profiles and a private space, built on Native Shell and the
distribution's WebKitGTK 6.0.

%prep
%autosetup -n tarn-browser-%{version}

%build
cd linux
cargo build --release --locked

%install
TARN_BIN=${CARGO_TARGET_DIR:-linux/target}/release/tarn sh linux/packaging/install.sh %{buildroot}

%files
%license LICENSE
%{_bindir}/tarn
%{_datadir}/applications/de.quavon.tarn.desktop
%{_datadir}/icons/hicolor/scalable/apps/de.quavon.tarn.svg
%exclude %{_datadir}/licenses/tarn

FROM fedora:43@sha256:f01112b9a1f1d0504ad962d9adfa401425033f8ab067e0a821904ebdde5b8ca5
RUN dnf -y install gcc gcc-c++ clang pkgconf-pkg-config git curl perl make openssl \
    gtk4-devel webkitgtk6.0-devel pango-devel cairo-devel glib2-devel graphene-devel \
    gstreamer1-devel gstreamer1-plugins-base-devel rpm-build \
    xorg-x11-server-Xvfb mesa-dri-drivers ImageMagick xdotool dbus-daemon python3 \
    at-spi2-core python3-gobject openbox ibus ibus-gtk4 procps-ng sqlite attr rustup && dnf clean all
RUN rustup-init -y --profile minimal --default-toolchain 1.99.1
ENV PATH=/root/.cargo/bin:$PATH

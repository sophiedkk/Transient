# Meson builds and installs Transient: it runs Cargo for the binary and installs
# everything around it (desktop file, metainfo, icons, settings schema, translations).
# Day-to-day development uses `just run`; Meson is used by Flatpak and `just meson-test`.
project(
  'transient',
  version: '0.1.0',
  meson_version: '>= 1.4.0',
  license: 'MIT',
)

i18n = import('i18n')
gnome = import('gnome')

base_id = 'io.github.sophiedkk.Transient'
if get_option('profile') == 'development'
  application_id = base_id + '.Devel'
else
  application_id = base_id
endif

# Fail early with a clear message if a library is missing or too old.
dependency('gtk4', version: '>= 4.20')
dependency('libadwaita-1', version: '>= 1.8')
dependency('gstreamer-1.0', version: '>= 1.24')
find_program('blueprint-compiler')
find_program('glib-compile-resources')
cargo = find_program('cargo')

prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
datadir = prefix / get_option('datadir')
localedir = prefix / get_option('localedir')
gettext_package = meson.project_name()
podir = meson.project_source_root() / 'po'

subdir('data')
subdir('po')

# Build the binary with Cargo. The settings reach the code via build.rs.
# Cargo shares ./target with `cargo run`, so nothing is compiled twice.
cargo_profile = get_option('buildtype') == 'release' ? 'release' : 'debug'
custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  console: true,
  output: meson.project_name(),
  install: true,
  install_dir: bindir,
  env: {
    'TRANSIENT_APP_ID': application_id,
    'TRANSIENT_PROFILE': get_option('profile'),
    'TRANSIENT_LOCALEDIR': localedir,
  },
  command: [
    find_program('build-aux/cargo-build.sh'),
    cargo,
    meson.project_source_root(),
    cargo_profile,
    '@OUTPUT@',
  ],
)

gnome.post_install(
  glib_compile_schemas: true,
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)

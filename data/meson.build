conf = configuration_data()
conf.set('APP_ID', application_id)

# Desktop file: makes Transient appear in the app grid.
desktop_file = i18n.merge_file(
  type: 'desktop',
  input: configure_file(
    input: base_id + '.desktop.in.in',
    output: '@BASENAME@',
    configuration: conf,
  ),
  output: application_id + '.desktop',
  po_dir: podir,
  install: true,
  install_dir: datadir / 'applications',
)

# AppStream metainfo: the description shown in GNOME Software and on Flathub.
metainfo_file = i18n.merge_file(
  input: configure_file(
    input: base_id + '.metainfo.xml.in.in',
    output: '@BASENAME@',
    configuration: conf,
  ),
  output: application_id + '.metainfo.xml',
  po_dir: podir,
  install: true,
  install_dir: datadir / 'metainfo',
)

install_data(
  base_id + '.gschema.xml',
  install_dir: datadir / 'glib-2.0' / 'schemas',
)

subdir('icons')

# `meson test` checks that these files follow the freedesktop specifications.
desktop_file_validate = find_program('desktop-file-validate', required: false)
if desktop_file_validate.found()
  test('validate-desktop', desktop_file_validate, args: [desktop_file])
endif

appstreamcli = find_program('appstreamcli', required: false)
if appstreamcli.found()
  test(
    'validate-metainfo',
    appstreamcli,
    args: ['validate', '--no-net', '--explain', metainfo_file],
  )
endif

glib_compile_schemas = find_program('glib-compile-schemas', required: false)
if glib_compile_schemas.found()
  test(
    'validate-schema',
    glib_compile_schemas,
    args: ['--strict', '--dry-run', meson.current_source_dir()],
  )
endif

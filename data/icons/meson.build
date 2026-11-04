# Icons are installed under the app ID, so the .Devel build gets its own copy.
# Development builds get the striped icon, so the two installs are easy to tell apart.
install_data(
  'hicolor/scalable/apps' / application_id + '.svg',
  install_dir: datadir / 'icons' / 'hicolor' / 'scalable' / 'apps',
  rename: application_id + '.svg',
)

install_data(
  'hicolor/symbolic/apps' / base_id + '-symbolic.svg',
  install_dir: datadir / 'icons' / 'hicolor' / 'symbolic' / 'apps',
  rename: application_id + '-symbolic.svg',
)

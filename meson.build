project('tagsandblocks', 'c',
    license: 'MIT',
    license_files: 'LICENSE',
    default_options: [
        'debug=true', # false when released
        'strip=false', # true when released
        'b_lto=false', # true when released
        'b_ndebug=false', # true when released
        'optimization=1', # 3 or s when released
        'b_sanitize=leak', # none when released 
                           # Possible values: address, thread, undefined, memory, leak, address, none.
        'warning_level=3',
        'c_std=c99',
    ],
    version: '0.0.0'
)
dep_pangocairo = dependency('pangocairo')
dep_cairo = dependency('cairo')
dep_wayland_client = dependency('wayland-client')

mod_wayland = import('unstable-wayland')
xdg_shell = mod_wayland.scan_xml('protocols/xdg-shell.xml', client: true, public: false)
wlr_layer_shell = mod_wayland.scan_xml('protocols/wlr-layer-shell-unstable-v1.xml', client: true, public: false)
river_status = mod_wayland.scan_xml('protocols/river-status-unstable-v1.xml', client: true, public: false)

executable(meson.project_name(), 'tagsandblocks.c', xdg_shell, wlr_layer_shell, river_status,
    dependencies: [
        dep_pangocairo,
        dep_cairo,
        dep_wayland_client
    ]
)

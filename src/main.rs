mod blocks;
mod river_status_protocol;
mod useless;

use std::{
    fs::File,
    os::unix::prelude::AsFd,
    sync::{Arc, Mutex},
};

use crate::river_status_protocol::{
    zriver_output_status_v1, zriver_seat_status_v1, zriver_status_manager_v1,
};
use wayland_client::{
    delegate_noop,
    protocol::{
        wl_buffer::{self, WlBuffer},
        wl_compositor, wl_output, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, QueueHandle,
};

use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use pangocairo::cairo as cr;
use pangocairo::pango as pango;

const TYPICAL_OUTPUT_AMOUNT: usize = 3;
type OutputsContexts = Arc<Mutex<Vec<OutputContext>>>;

fn main() {
    let outputs_contexts: OutputsContexts =
        Arc::new(Mutex::new(Vec::with_capacity(TYPICAL_OUTPUT_AMOUNT)));

    use blocks::Block;
    use std::process::Command;
    let blocks = vec![
        Block {
            icon: String::from(""),
            command: Command::new("date"),
            interval: 1,
            signal: 1,
        },
        Block {
            icon: String::from("Happy face"),
            command: Command::new("/home/evgen/battery"),
            interval: 1,
            signal: 0,
        },
    ];
    blocks::setup_signals(&blocks);

    let conn = Arc::new(Connection::connect_to_env().unwrap());

    blocks::spawn_and_configure_blocks_updates_thread(
        blocks,
        Arc::clone(&outputs_contexts),
        Arc::clone(&conn),
    );
    let mut bar = Bar::new(Arc::clone(&outputs_contexts));

    let mut event_queue = conn.new_event_queue();
    let qhandle = event_queue.handle();

    let display = conn.display();
    display.get_registry(&qhandle, ());

    while bar.running {
        event_queue.blocking_dispatch(&mut bar).unwrap();
    }
}

pub struct OutputContext {
    ready_to_draw: bool,
    width: i32,
    height: i32,
    current_buffer_index: usize,
    surface: wl_surface::WlSurface,
    // Required to keep mmap from droping and let us send this to other thread cause cairo::Context didn't implement Send+Sync
    canvases: [memmap2::MmapMut; 2],
    buffers: [WlBuffer; 2],
}
struct Bar {
    ready_to_draw: bool,
    // SHould be purished
    running: bool,

    file: File,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    compositor: Option<wl_compositor::WlCompositor>,

    tags: Vec<u32>,
    focused_tag: u32,
    title: String,
    previous_tags_and_title_length: i32,

    // All subsequent variables depends on outputs in some way and therefore should be configured inside wl_output's events
    outputs: Vec<wl_output::WlOutput>,
    pool: Option<(wl_shm_pool::WlShmPool, i32)>,
    layer_surfaces: Vec<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    outputs_contexts: OutputsContexts,
}

impl Bar {
    fn new(outputs_contexts: OutputsContexts) -> Self {
        let mut tags = Vec::with_capacity(9);
        tags.push(1);

        let file = std::fs::File::options()
                .create(true)
                .write(true)
                .read(true)
                .open("./shared_memory_file")
                .unwrap();
            file.set_len(2u64.pow(30)).unwrap();
        Self {
            ready_to_draw: false,
            running: true,
            file,
            shm: None,
            layer_shell: None,
            compositor: None,
            tags,
            focused_tag: 1,
            title: String::from("Have a nice day!"),
            previous_tags_and_title_length: -1,
            outputs: Vec::with_capacity(3),
            pool: None,
            layer_surfaces: Vec::with_capacity(3),
            outputs_contexts,
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for Bar {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        static mut RIVER_STATUS_MANAGER: Option<zriver_status_manager_v1::ZriverStatusManagerV1> =
            None;
        static mut SEAT: Option<wl_seat::WlSeat> = None;
        static mut RIVER_SEAT_STATUS: Option<zriver_seat_status_v1::ZriverSeatStatusV1> = None;
        static mut RIVER_OUTPUT_STATUSES: Option<
            Vec<zriver_output_status_v1::ZriverOutputStatusV1>,
        > = None;
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                match &interface[..] {
                    "wl_shm" => {
                        state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
                    }
                    "zwlr_layer_shell_v1" => {
                        state.layer_shell = Some(
                            registry.bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(
                                name,
                                1,
                                qh,
                                (),
                            ),
                        );
                    }
                    "wl_compositor" => {
                        state.compositor = Some(
                            registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ()),
                        );
                    }
                    // Trust me
                    "zriver_status_manager_v1" => unsafe {
                        RIVER_STATUS_MANAGER = Some(
                            registry.bind::<zriver_status_manager_v1::ZriverStatusManagerV1, _, _>(
                                name,
                                version,
                                qh,
                                (),
                            ),
                        );
                        if RIVER_OUTPUT_STATUSES.is_none() {
                            RIVER_OUTPUT_STATUSES = Some(Vec::with_capacity(3));
                        }
                    },
                    // Trust me
                    "wl_seat" => unsafe {
                        SEAT = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version, qh, ()));
                    },
                    // Trust me
                    "wl_output" => unsafe {
                        let output: wl_output::WlOutput = registry.bind(name, version, qh, ());

                        // May be errornous to assume that output will be the last objects in the sequence of events
                        RIVER_OUTPUT_STATUSES.as_mut().unwrap().push(
                            RIVER_STATUS_MANAGER
                                .as_ref()
                                .unwrap()
                                .get_river_output_status(&output, qh, ()),
                        );

                        state.outputs.push(output);
                    },
                    _ => (),
                }
            }
            wl_registry::Event::GlobalRemove { name: _ } => {
                // Implement output removal
            }
            _ => (),
        }
        // Optimize later to not process this every event
        unsafe {
            if RIVER_SEAT_STATUS.is_none() && SEAT.is_some() && RIVER_STATUS_MANAGER.is_some() {
                RIVER_SEAT_STATUS = Some(
                    RIVER_STATUS_MANAGER
                        .as_ref()
                        .unwrap()
                        .get_river_seat_status(SEAT.as_ref().unwrap(), qh, ()),
                );
            }
        }
    }
}

// Ignore events from these object types in this example.
delegate_noop!(Bar: ignore wl_compositor::WlCompositor);
delegate_noop!(Bar: ignore wl_surface::WlSurface);
delegate_noop!(Bar: ignore wl_shm::WlShm);
delegate_noop!(Bar: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Bar: ignore wl_buffer::WlBuffer);
delegate_noop!(Bar: ignore zriver_status_manager_v1::ZriverStatusManagerV1);
delegate_noop!(Bar: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

impl Bar {
    fn draw_tags_and_title(&mut self) {
        if self.ready_to_draw == false {
            return ();
        }
        const BLOCK_WIDTH_PROCENT: f64 = 0.015;

        let mut outputs_contexts = self.outputs_contexts.lock().unwrap();

        for output_context in outputs_contexts.iter_mut() {
            let width = output_context.width;
            let height = output_context.height;
            let surface = &output_context.surface;
            let buffer = &output_context.buffers[output_context.current_buffer_index];
            let mmap_ptr =
                output_context.canvases[output_context.current_buffer_index].as_mut_ptr();
            let image_surface = unsafe {
                cr::ImageSurface::create_for_data_unsafe(
                    mmap_ptr,
                    cr::Format::ARgb32,
                    width,
                    height,
                    width * 4,
                )
                .unwrap()
            };
            let cr = cr::Context::new(image_surface).unwrap();

            let block_width = (width as f64 * BLOCK_WIDTH_PROCENT) as i32;

            let mut tags = self.tags.clone();
            if !self.tags.contains(&self.focused_tag) {
                tags.push(self.focused_tag);
                tags.sort();
            }
            let pg_layout = pangocairo::create_layout(&cr);
            let mut font_desc = pango::FontDescription::new();
            font_desc.set_family("IosevkaNerdFontMono");
            font_desc.set_weight(pango::Weight::Bold);
            font_desc.set_style(pango::Style::Normal);
            pg_layout.set_font_description(Some(&font_desc));

            pg_layout.set_text(&self.title);

            if self.previous_tags_and_title_length == -1 {
                self.previous_tags_and_title_length =
                    block_width * tags.len() as i32 + pg_layout.pixel_size().0;
                dbg!(pg_layout.pixel_size().0);
                cr.set_source_rgb(0.0, 0.0, 0.0);
                cr.rectangle(0.0, 0.0, width as f64, height as f64);
                cr.fill().unwrap();
            }
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.rectangle(
                0.0,
                0.0,
                self.previous_tags_and_title_length as f64,
                height as f64,
            );
            cr.fill().unwrap();

            cr.set_source_rgb(0.0, 1.0, 0.0);
            cr.move_to(block_width as f64 * tags.len() as f64, 0.);
            pangocairo::show_layout(&cr, &pg_layout);

            for i in 0..tags.len() {
                let tag_i = tags[i];
                if tag_i == self.focused_tag {
                    cr.set_source_rgb(0.0, 0.0, 1.0);
                } else {
                    cr.set_source_rgb(1., 1., 1.);
                }
                let offset = block_width as f64 * i as f64;
                cr.rectangle(offset, 0.0, 20., 20.);
                cr.fill().unwrap();

                let tag_i_pos = bitflag_to_pos(tag_i);

                cr.set_source_rgb(0.0, 0.0, 0.0);
                cr.move_to(block_width as f64 * i as f64, 0.);
                pg_layout.set_text(format!("{tag_i_pos}").as_str());
                pangocairo::show_layout(&cr, &pg_layout);
            }

            surface.attach(Some(buffer), 0, 0);
            surface.damage(0, 0, self.previous_tags_and_title_length, height);
            surface.commit();

            pg_layout.set_text(&self.title);
            self.previous_tags_and_title_length =
                block_width * tags.len() as i32 + pg_layout.pixel_size().0;

            output_context.current_buffer_index += 1;
            output_context.current_buffer_index %= 2;
        }
    }
}

impl Dispatch<zriver_seat_status_v1::ZriverSeatStatusV1, ()> for Bar {
    fn event(
        state: &mut Self,
        _: &zriver_seat_status_v1::ZriverSeatStatusV1,
        event: <zriver_seat_status_v1::ZriverSeatStatusV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zriver_seat_status_v1::Event::FocusedView { title } = event {
            state.title = title;
            state.draw_tags_and_title();
        }
    }
}

impl Dispatch<zriver_output_status_v1::ZriverOutputStatusV1, ()> for Bar {
    fn event(
        state: &mut Self,
        _: &zriver_output_status_v1::ZriverOutputStatusV1,
        event: <zriver_output_status_v1::ZriverOutputStatusV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zriver_output_status_v1::Event;
        match event {
            Event::FocusedTags { tags } => {
                state.focused_tag = tags;
                state.draw_tags_and_title();
            }
            Event::ViewTags { tags } => {
                let mut tags: Vec<u32> = tags
                    .chunks_exact(4)
                    .map(|bytes_4| u32::from_ne_bytes(bytes_4.try_into().unwrap()))
                    .collect();
                tags.sort();
                tags.dedup();
                state.tags = tags;
                state.draw_tags_and_title();
            }
            _ => (),
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for Bar {
    fn event(
        state: &mut Self,
        proxy: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: <zwlr_layer_surface_v1::ZwlrLayerSurfaceV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_layer_surface_v1::Event;
        match event {
            Event::Configure {
                serial,
                width: _,
                height: _,
            } => {
                let mut output_index = usize::MAX;
                for i in 0..state.layer_surfaces.len() {
                    if &state.layer_surfaces[i] == proxy {
                        output_index = i;
                    }
                }

                state
                    .layer_surfaces
                    .get(output_index)
                    .unwrap()
                    .ack_configure(serial);
                // Should be purished
                state.ready_to_draw = true;
                let mut outputs_contexts = state.outputs_contexts.lock().unwrap();
                outputs_contexts[output_index].ready_to_draw = true;
                drop(outputs_contexts);
                state.draw_tags_and_title();
            }
            Event::Closed => {
                println!("Surface should be closed");
            }
            _ => (),
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Bar {
    fn event(
        state: &mut Self,
        _proxy: &wl_output::WlOutput,
        event: <wl_output::WlOutput as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // It maybe errornous to assume that first event will be mode and then scale. Rework later
        const HEIGHT_PROCENT: f32 = 0.015;
        if let wl_output::Event::Scale { factor: _ } = event {
            // Fuck it. I will ignore the scale
        } else if let wl_output::Event::Mode {
            flags: _,
            width,
            height: hght,
            refresh: _,
        } = event
        {
            let height = (hght as f32 * HEIGHT_PROCENT) as i32;
            if state.shm.is_none() || state.layer_shell.is_none() {
                panic!("Got wl_output object before wl_shm");
            }
            let surface = state
                .compositor
                .as_ref()
                .expect("shiiiit. Got output before compositor")
                .create_surface(qh, ());
            if state.pool.is_none() {
                let pool_size = (width * height * 4) * 2;
                state.pool = Some((
                    state
                        .shm
                        .as_ref()
                        .unwrap()
                        .create_pool(state.file.as_fd(), pool_size, qh, ()),
                    pool_size,
                ));

                let buffer1 = state.pool.as_ref().unwrap().0.create_buffer(
                    0,
                    width,
                    height,
                    width * 4,
                    wl_shm::Format::Argb8888,
                    qh,
                    (),
                );
                let mmap1 = unsafe {
                    memmap2::MmapOptions::new()
                        .offset((0) as u64)
                        .len((width * 4 * height) as usize)
                        .map_mut(&state.file)
                        .unwrap()
                };

                let buffer2 = state.pool.as_ref().unwrap().0.create_buffer(
                    width * 4 * height,
                    width,
                    height,
                    width * 4,
                    wl_shm::Format::Argb8888,
                    qh,
                    (),
                );
                let mmap2 = unsafe {
                    memmap2::MmapOptions::new()
                        .offset((width * 4 * height) as u64)
                        .len((width * 4 * height) as usize)
                        .map_mut(&state.file)
                        .unwrap()
                };

                let layer_surface = state.layer_shell.as_ref().unwrap().get_layer_surface(
                    &surface,
                    None,
                    zwlr_layer_shell_v1::Layer::Top,
                    "statusbar".to_string(),
                    qh,
                    (),
                );
                // Layer configure
                use zwlr_layer_surface_v1::Anchor;
                layer_surface.set_anchor(Anchor::Top);
                layer_surface.set_exclusive_zone(height);
                use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::KeyboardInteractivity;
                layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
                layer_surface.set_size(width as u32, height as u32);
                surface.commit();

                state.layer_surfaces.push(layer_surface);

                let mut draw_contexts = state.outputs_contexts.lock().unwrap();
                draw_contexts.push(OutputContext {
                    ready_to_draw: false,
                    width,
                    height,
                    current_buffer_index: 0,
                    surface,
                    canvases: [mmap1, mmap2],
                    buffers: [buffer1, buffer2],
                });
            } else {
                // This does not catches the case where output is unused and memory corresponding to this output no longer in use
                let new_size = state.pool.as_ref().unwrap().1 + width * height * 4 * 2;
                state.pool.as_ref().unwrap().0.resize(new_size);
                // Add buffers
                todo!();
                // Add layer surface
            }
        }
    }
}

fn bitflag_to_pos(mut bitflag: u32) -> u32 {
    let mut pos = 0;
    while bitflag != 0 {
        bitflag >>= 1;
        pos += 1;
    }
    pos
}

#[test]
fn test_bitflag() {
    assert_eq!(bitflag_to_pos(0b1), 1);
    assert_eq!(bitflag_to_pos(0b100), 3);
}

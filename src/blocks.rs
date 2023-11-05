use std::os::unix::ffi::OsStringExt;
use std::{
    ffi::OsString,
    sync::{Arc, Mutex},
};

use crate::{OutputContext, OutputsContexts};

fn gcd(mut x: i32, mut y: i32) -> i32 {
    let mut temp;
    while y > 0 {
        temp = x % y;

        x = y;
        y = temp;
    }
    return x;
}

pub struct Block {
    pub icon: String,
    pub command: std::process::Command,
    pub interval: u32,
    pub signal: libc::c_int,
}

type SignalFD = libc::c_int;

impl Block {
    pub fn run_and_get_output(&mut self) -> OsString {
        let output = self.command.output().unwrap().stdout;
        OsString::from_vec(output)
    }
}

pub fn spawn_and_configure_blocks_updates_thread(
    mut blocks: Vec<Block>,
    mut draw_contexts: Arc<Mutex<Vec<OutputContext>>>,
    conn: Arc<Connection>,
) {
    std::thread::spawn(move || {
        let mut block_outputs: Vec<OsString> = Vec::with_capacity(blocks.len());
        for block in blocks.iter_mut() {
            block_outputs.push(block.run_and_get_output());
        }
        let signal_fd = setup_signals(&blocks);
        let mut timer_interval = -1;
        for i in 0..blocks.len() {
            if blocks[i].interval > 0 {
                // Check user config correctness please
                timer_interval = gcd(blocks[i].interval as i32, timer_interval);
            }
        }
        unsafe { libc::raise(libc::SIGALRM) };
        let mut pfd = [libc::pollfd {
            fd: signal_fd,
            events: libc::POLLIN,
            revents: 0,
        }];

        let mut time = 0;
        // Add running var
        loop {
            // Wait for new signal (poll() blocks thread)
            // fuck this as casting
            let poll_result = unsafe {
                libc::poll(
                    &mut pfd as *mut _,
                    (std::mem::size_of_val(&pfd) / std::mem::size_of::<libc::pollfd>()) as u64,
                    -1,
                )
            };
            if poll_result < 0 || (!(pfd[0].revents & libc::POLLIN) > 0) {
                break;
            };
            let mut signal_info: libc::signalfd_siginfo = unsafe { std::mem::zeroed() };
            let _read_result = unsafe {
                libc::read(
                    signal_fd,
                    &mut signal_info as *mut _ as *mut libc::c_void,
                    std::mem::size_of::<libc::signalfd_siginfo>(),
                )
            };
            //if read_result < 0 {
            //    dbg!("signal handling failed");
            //}
            match signal_info.ssi_signo as i32 {
                libc::SIGALRM => unsafe {
                    libc::alarm(timer_interval as u32);
                    for (i, block) in blocks.iter_mut().enumerate() {
                        if block.interval == 0 {
                            continue;
                        }
                        if time % block.interval == 0 {
                            block_outputs[i] = block.run_and_get_output();
                        }
                    }
                    time += timer_interval as u32;
                },
                libc::SIGUSR1 => {
                    // Maybe there should be some system that will check for
                    // user pointer and send corresponding
                    // event but I don't know how that should work
                    // A have nothing to deal with this for now
                    return;
                }
                signal => {
                    for (i, block) in blocks.iter_mut().enumerate() {
                        if block.signal == signal {
                            block_outputs[i] = block.run_and_get_output();
                        }
                    }
                }
            }
            display_blocks(&block_outputs, &mut draw_contexts);
            conn.flush().unwrap();
        }
    });
}

pub fn setup_signals(blocks: &[Block]) -> SignalFD {
    unsafe {
        let mut signals: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut signals as *mut _);
        // Process time based events
        libc::sigaddset(&mut signals as *mut _, libc::SIGALRM);
        // Process button events (todo!())
        libc::sigaddset(&mut signals as *mut _, libc::SIGUSR1);

        // Process all signals decalred in blocks
        for i in 0..blocks.len() {
            if blocks[i].signal > 0 {
                libc::sigaddset(&mut signals as *mut _, libc::SIGRTMIN() + blocks[i].signal);
            }
        }
        // Create signal file descriptor for pooling
        let signal_fd = libc::signalfd(-1, &signals, 0);

        // Block previous signals and all other RealTime events
        for i in libc::SIGRTMIN()..=libc::SIGRTMAX() {
            libc::sigaddset(&mut signals as *mut _, i);
        }
        libc::sigprocmask(libc::SIG_BLOCK, &mut signals as *mut _, 0 as *mut _);
        signal_fd
    }
}
use pangocairo::cairo as cr;
use pangocairo::pango as pango;
use wayland_client::Connection;
fn display_blocks(block_outputs: &[OsString], outputs_contexts: &mut OutputsContexts) {
    let mut outputs_contexts = outputs_contexts.lock().unwrap();

    for output_index in 0..outputs_contexts.len() {
        let output_context = outputs_contexts.get_mut(output_index).unwrap();
        if output_context.ready_to_draw == false {
            continue;
        }
        let width = output_context.width;
        let height = output_context.height;
        let surface = &output_context.surface;
        let buffer = output_context
            .buffers
            .get(output_context.current_buffer_index)
            .unwrap();
        let mmap_ptr = output_context.canvases[output_context.current_buffer_index].as_mut_ptr();
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

        let pg_layout = pangocairo::create_layout(&cr);
        let mut font_desc = pango::FontDescription::new();
        font_desc.set_family("IosevkaNerdFontMono");
        font_desc.set_weight(pango::Weight::Bold);
        font_desc.set_style(pango::Style::Normal);
        pg_layout.set_font_description(Some(&font_desc));
        let mut previous_offset = output_context.width;
        for block_output in block_outputs {
            //cr.rectangle(offset, 0.0, 20., 20.);
            //cr.fill().unwrap();
            pg_layout.set_text(block_output.to_str().unwrap().trim());
            let offset = previous_offset - pg_layout.pixel_size().0 - 10;

            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.rectangle(
                offset as f64,
                0.0,
                pg_layout.pixel_size().0 as f64,
                height as f64,
            );
            cr.fill().unwrap();

            cr.set_source_rgb(1., 0.0, 0.);
            cr.move_to(offset as f64, 0.);
            pangocairo::show_layout(&cr, &pg_layout);
            previous_offset = offset;
            //cr.select_font_face("monospace", cr::FontSlant::Normal, cr::FontWeight::Bold);
            //cr.set_font_size(1.2);

            //cr.text_extents("1").unwrap();
            //cr.show_text("1").unwrap();
        }

        surface.attach(Some(buffer), 0, 0);
        surface.damage(previous_offset, 0, width - previous_offset, height);
        surface.commit();

        outputs_contexts
            .get_mut(output_index)
            .unwrap()
            .current_buffer_index += 1;
        outputs_contexts
            .get_mut(output_index)
            .unwrap()
            .current_buffer_index %= 2;
    }
}

use std::io::Write;

use gst::prelude::*;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct Args {
    #[structopt(long)]
    live_rtmp_uri: String,
    #[structopt(long, help = "Make RTMP pipeline EOS after N buffers")]
    eos_after: Option<i32>,
    #[structopt(long, help = "Make RTMP pipeline error after N buffers")]
    error_after: Option<i32>,
    #[structopt(long, help = "Make compositor discard RTMP buffers after N seconds")]
    discard_after: Option<u64>,
}

fn default_handle_message(pipe: &gst::Pipeline, msg: &gst::Message) {
    match msg.view() {
        gst::MessageView::Latency(..) => {
            println!("Recalculating latency!");
            pipe.recalculate_latency().unwrap();
        }
        gst::MessageView::StateChanged(state_changed) => {
            if state_changed.get_src().map(|s| &s == pipe).unwrap_or(false)
                && state_changed.get_current() == gst::State::Playing
            {
                pipe.debug_to_dot_file(
                    gst::DebugGraphDetails::all(),
                    format!("PLAYING-{}", pipe.get_name()),
                );
            }
        }
        _ => (),
    }
}

fn build_rtmp_pipeline(args: &Args) -> Result<gst::Pipeline, anyhow::Error> {
    let playbin = gst::ElementFactory::make("playbin3", Some("rtmp_source"))?;
    let vsink = gst::parse_bin_from_description(
        "identity name=id ! interpipesink drop=false sync=true name=rtmp",
        true,
    )?;
    let asink = gst::ElementFactory::make("fakesink", None)?;

    let identity = vsink.get_by_name("id").unwrap();

    if let Some(eos_after) = args.eos_after {
        identity.set_property("eos-after", &eos_after)?;
    }

    if let Some(error_after) = args.error_after {
        identity.set_property("error-after", &error_after)?;
    }

    playbin.set_property("uri", &args.live_rtmp_uri)?;
    playbin.set_property("video-sink", &vsink)?;
    playbin.set_property("audio-sink", &asink)?;

    let pipe = playbin.downcast::<gst::Pipeline>().unwrap();
    let bus = pipe.get_bus().unwrap();
    let pipe_clone = pipe.clone();
    let uri = args.live_rtmp_uri.clone();

    bus.add_watch(move |_, msg| {
        let pipe = &pipe_clone;
        match msg.view() {
            gst::MessageView::Error(err) => {
                /* Naive throttling */
                std::thread::sleep(std::time::Duration::from_millis(1000));
                eprintln!("Error: {:?}, restarting pipeline", err);
                restart_pipeline(uri.clone(), pipe);
            }
            gst::MessageView::Buffering(buffering) => {
                let percent = buffering.get_percent();
                print!("Buffering ({}%)\r", percent);
                match std::io::stdout().flush() {
                    Ok(_) => {}
                    Err(err) => eprintln!("Failed: {}", err),
                };

                if percent < 100 {
                    let _ = pipe.set_state(gst::State::Paused);
                } else {
                    let _ = pipe.set_state(gst::State::Playing);
                }
            }
            gst::MessageView::Eos(_) => {
                eprintln!("We are EOS");
                restart_pipeline(uri.clone(), pipe);
            }
            _ => default_handle_message(pipe, msg),
        };
        glib::Continue(true)
    })?;

    Ok(pipe)
}

fn build_compositor_pipeline(args: &Args) -> Result<gst::Pipeline, anyhow::Error> {
    let pipe = gst::Pipeline::new(Some("video_mixer"));

    let interpipesrc = gst::ElementFactory::make("interpipesrc", None)?;
    let queue = gst::ElementFactory::make("queue", None)?;
    let compositor = gst::ElementFactory::make("compositor", None)?;
    let sink = gst::ElementFactory::make("xvimagesink", None)?;

    pipe.add_many(&[&interpipesrc, &queue, &compositor, &sink])?;

    gst::Element::link_many(&[&interpipesrc, &queue, &compositor, &sink])?;

    let pad = compositor.get_static_pad("sink_0").unwrap();
    pad.set_property("zorder", &(1 as u32))?;
    pad.set_property("width", &1280)?;
    pad.set_property("height", &720)?;

    if let Some(discard_after) = args.discard_after {
        pad.set_property("max-last-buffer-repeat", &(discard_after * gst::SECOND))?;
    }

    interpipesrc.set_property("listen-to", &"rtmp")?;
    interpipesrc.set_property("format", &gst::Format::Time)?;
    interpipesrc.set_property("is-live", &true)?;
    interpipesrc.set_property_from_str("stream-sync", &"restart-ts");

    // FIXME: interpipesink should translate QoS events when stream-sync = compensate-ts
    sink.set_property("qos", &false).unwrap();

    let fallbacksrc = gst::ElementFactory::make("videotestsrc", None)?;
    let queue = gst::ElementFactory::make("queue", None)?;
    let capsfilter = gst::ElementFactory::make("capsfilter", None)?;

    fallbacksrc.set_property("is-live", &true)?;
    capsfilter.set_property(
        "caps",
        &gst::Caps::new_simple("video/x-raw", &[("width", &800), ("height", &448)]),
    )?;

    pipe.add_many(&[&fallbacksrc, &queue, &capsfilter])?;
    gst::Element::link_many(&[&fallbacksrc, &queue, &capsfilter, &compositor])?;

    let pad = compositor.get_static_pad("sink_1").unwrap();
    pad.set_property("zorder", &(0 as u32))?;
    pad.set_property("width", &1280)?;
    pad.set_property("height", &720)?;

    let bus = pipe.get_bus().unwrap();
    let pipe_clone = pipe.clone();
    bus.add_watch(move |_, msg| {
        let pipe = &pipe_clone;
        default_handle_message(pipe, msg);
        glib::Continue(true)
    })?;

    Ok(pipe)
}

fn restart_pipeline(uri: String, pipe: &gst::Pipeline) {
    pipe.set_state(gst::State::Null).unwrap();
    pipe.set_property("uri", &uri).unwrap();
    pipe.set_state(gst::State::Playing).unwrap();
}

fn main() -> Result<(), anyhow::Error> {
    gst::init()?;

    let args = Args::from_args();

    let rtmp_pipe = build_rtmp_pipeline(&args)?;
    let compositor_pipe = build_compositor_pipeline(&args)?;

    rtmp_pipe.set_state(gst::State::Playing)?;
    compositor_pipe.set_state(gst::State::Playing)?;

    let main_loop = glib::MainLoop::new(None, false);

    main_loop.run();

    rtmp_pipe.set_state(gst::State::Null)?;
    compositor_pipe.set_state(gst::State::Null)?;

    Ok(())
}

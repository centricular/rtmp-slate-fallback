# RTMP slate fallback

This is an example on how to fallback to a "slate" stream using compositor.

In this example, we use a simple videotestsrc as the "slate" stream, it can
of course be replaced with any other reliable source.

This example has been tested with live RTMP streams, and uses interpipe
elements to isolate the source and mixing pipelines.

This requires patches from <https://gitlab.freedesktop.org/gstreamer/gst-plugins-base/-/merge_requests/620>

Build with:

``` shell
cargo build
```

Display help:

``` shell
cargo run -- --help
```

## Behaviour on error in the source pipeline

Test with:

```
cargo run -- --live-rtmp-uri rtmp://192.168.1.107:1935/live/myStreamd --error-after 300 --discard-after 2
```

Expected behaviour:

* the slate should be displayed while the RTMP pipeline is buffering

* then 300 buffers (10 seconds with a 30 fps framerate) should be displayed from the live RTMP stream.

* the last buffer should stay displayed for 2 seconds

* the fallback slate should be displayed again while the source pipeline rebuffers

* As the bus handler restarts the pipeline upon error, the RTMP stream should be displayed again after buffering

You can change `--discard-after` to 0 to fall back to the slate without freezing,
not specifying it will freeze the last received buffer for ever, which means
the slate will only be displayed at the start.

## Behaviour on EOS from the source pipeline

Test with:

```
cargo run -- --live-rtmp-uri rtmp://192.168.1.107:1935/live/myStreamd --eos-after 300 --discard-after 2
```

When the input stream terminates normally, the last buffer should not be displayed
longer than its normal duration. This can be verified with the above pipeline.

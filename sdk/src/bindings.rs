//! Generated from `wit/`. Apps don't use these directly; the modules at the
//! crate root wrap them, and `app!` exports the handlers.
//!
//! One `generate!` per export: with `pub_export_macro`, a world's exports
//! are declared by its export macro in the app's crate, not here, so an app's
//! component exports `start` and exactly the handlers it passes to `app!`;
//! the host looks them up (docs/abi.md, "Exports").

wit_bindgen::generate!({
    path: "../wit",
    world: "platform:app/start-app",
    generate_all,
    pub_export_macro: true,
    export_macro_name: "export_start",
    default_bindings_module: "islas_sdk::bindings",
});

pub mod http {
    wit_bindgen::generate!({
        path: "../wit",
        world: "platform:app/http-app",
        with: {
            "wasi:http/types@0.2.12": crate::bindings::wasi::http::types,
            "wasi:io/error@0.2.12": crate::bindings::wasi::io::error,
            "wasi:io/poll@0.2.12": crate::bindings::wasi::io::poll,
            "wasi:io/streams@0.2.12": crate::bindings::wasi::io::streams,
            "wasi:clocks/monotonic-clock@0.2.12": crate::bindings::wasi::clocks::monotonic_clock,
        },
        pub_export_macro: true,
        export_macro_name: "export_http",
        default_bindings_module: "islas_sdk::bindings::http",
    });
}

pub mod ws {
    wit_bindgen::generate!({
        path: "../wit",
        world: "platform:app/ws-app",
        with: { "platform:app/types@0.1.0": crate::bindings::platform::app::types },
        pub_export_macro: true,
        export_macro_name: "export_ws",
        default_bindings_module: "islas_sdk::bindings::ws",
    });
}

pub mod stream {
    wit_bindgen::generate!({
        path: "../wit",
        world: "platform:app/stream-app",
        with: { "platform:app/types@0.1.0": crate::bindings::platform::app::types },
        pub_export_macro: true,
        export_macro_name: "export_stream",
        default_bindings_module: "islas_sdk::bindings::stream",
    });
}

pub mod job {
    wit_bindgen::generate!({
        path: "../wit",
        world: "platform:app/job-app",
        pub_export_macro: true,
        export_macro_name: "export_job",
        default_bindings_module: "islas_sdk::bindings::job",
    });
}

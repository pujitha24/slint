// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use i_slint_core::InternalToken;

/// The case at `path`, compiled like the generated embed-assets cases and then sent through
/// `CompilationResultSend::to_bytes()`, as a microcontroller receives it.
pub fn load_through_bytes(path: &str, scale_factor: f32) -> slint_interpreter::ComponentInstance {
    // The build script sets these for the compiler it runs; this one runs now.
    // Safety: the test threads only read these, and all set the same values.
    unsafe {
        std::env::set_var("SLINT_DEFAULT_FONT", env!("SLINT_DEFAULT_FONT"));
        std::env::set_var("SLINT_FONT_PATH", env!("SLINT_FONT_PATH"));
    }
    let source = std::fs::read_to_string(path).unwrap();
    let mut compiler = slint_interpreter::Compiler::default();
    compiler.set_style("fluent".into());
    compiler.set_include_paths(
        test_driver_lib_include_paths(&source).into_iter().map(Into::into).collect(),
    );
    compiler.set_embed_resources(slint_interpreter::EmbedResourcesKind::EmbedTextures);
    let config = compiler.compiler_configuration(InternalToken);
    config.enable_experimental = true;
    config.const_scale_factor = Some(scale_factor);
    let result = spin_on_ready(compiler.build_from_source(source, path.into()));
    result.print_diagnostics();
    assert!(!result.has_errors(), "compile error in {path}");

    let bytes = result.into_send().to_bytes(InternalToken).unwrap();
    let received: slint_interpreter::CompilationResult =
        slint_interpreter::CompilationResultSend::from_bytes(&bytes, InternalToken).unwrap().into();
    received.component("TestCase").expect("no TestCase component").create().unwrap()
}

/// The `//include_path: ` markers of a case.
fn test_driver_lib_include_paths(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("//include_path:"))
        .map(|path| path.trim().to_string())
        .collect()
}

fn spin_on_ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    let waker = std::task::Waker::noop();
    match future.as_mut().poll(&mut std::task::Context::from_waker(waker)) {
        std::task::Poll::Ready(result) => result,
        std::task::Poll::Pending => panic!("the compiler waited on a file loader"),
    }
}

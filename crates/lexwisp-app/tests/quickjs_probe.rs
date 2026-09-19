//! Stage 0 dependency gates only. No VM or probe binary ships with the UI.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use rquickjs::{Context, Runtime};

#[test]
fn quickjs_returns_unicode_and_interrupts_a_synchronous_loop() {
    let runtime = Runtime::new().expect("create QuickJS runtime");
    runtime.set_memory_limit(64 * 1024 * 1024);
    runtime.set_max_stack_size(512 * 1024);
    let context = Context::full(&runtime).expect("create QuickJS context");
    context.with(|cx| {
        assert_eq!(
            cx.eval::<String, _>("'中文 ' + (6 * 7)").unwrap(),
            "中文 42"
        );
    });
    let interrupted = Arc::new(AtomicBool::new(false));
    let observed = interrupted.clone();
    let started = Instant::now();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        let expired = started.elapsed() >= Duration::from_millis(50);
        if expired {
            observed.store(true, Ordering::Relaxed);
        }
        expired
    })));
    context.with(|cx| {
        let error = cx
            .eval::<(), _>("for (;;) {}")
            .expect_err("loop must be interrupted");
        assert!(error.is_exception());
        let exception = cx.catch();
        assert!(format!("{exception:?}").contains("interrupt"));
    });
    assert!(interrupted.load(Ordering::Relaxed));
    assert!(started.elapsed() < Duration::from_secs(2));
    eprintln!(
        "QuickJS synchronous loop interrupted after {:?}",
        started.elapsed()
    );
    runtime.set_interrupt_handler(None);
    context.with(|cx| assert_eq!(cx.eval::<i32, _>("21 * 2").unwrap(), 42));
}

#[test]
fn quickjs_default_allocator_enforces_memory_limit() {
    let runtime = Runtime::new().unwrap();
    runtime.set_memory_limit(8 * 1024 * 1024);
    let context = Context::full(&runtime).unwrap();
    let started = Instant::now();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        started.elapsed() > Duration::from_secs(2)
    })));
    context.with(|cx| {
        let error = cx
            .eval::<(), _>("globalThis.data = new ArrayBuffer(32 * 1024 * 1024)")
            .expect_err("allocation over the VM limit must fail");
        assert!(error.is_exception());
        let exception = cx.catch();
        assert!(format!("{exception:?}").contains("memory"));
    });
}

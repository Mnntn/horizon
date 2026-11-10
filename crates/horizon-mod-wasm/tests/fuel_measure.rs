//! Standalone guest-work measurement, not a runtime acceptance test.
#[test]
#[ignore = "measurement"]
fn million_operations() -> wasmtime::Result<()> {
    use std::time::Instant;
    let wasm = wat::parse_str(
        r#"(module (func (export "run") (result i32)
        (local $i i32) (local $x i32)
        (loop $loop
            local.get $x i32.const 1664525 i32.mul i32.const 1013904223 i32.add local.set $x
            local.get $i i32.const 1 i32.add local.tee $i i32.const 1000000 i32.lt_u br_if $loop)
        local.get $x))"#,
    )?;
    for fuel in [false, true] {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(fuel).epoch_interruption(!fuel);
        let engine = wasmtime::Engine::new(&config)?;
        let module = wasmtime::Module::new(&engine, &wasm)?;
        let mut store = wasmtime::Store::new(&engine, ());
        if fuel {
            store.set_fuel(1_000_000_000)?;
        } else {
            store.set_epoch_deadline(100);
        }
        let instance = wasmtime::Instance::new(&mut store, &module, &[])?;
        let run = instance.get_typed_func::<(), i32>(&mut store, "run")?;
        for _ in 0..10 {
            if fuel {
                store.set_fuel(1_000_000_000)?;
            }
            run.call(&mut store, ())?;
        }
        for rep in 0..3 {
            let mut times = Vec::new();
            let mut used = 0;
            for _ in 0..100 {
                if fuel {
                    store.set_fuel(1_000_000_000)?;
                }
                let start = Instant::now();
                let result = run.call(&mut store, ())?;
                times.push(start.elapsed().as_nanos());
                if fuel {
                    used = 1_000_000_000 - store.get_fuel()?;
                }
                std::hint::black_box(result);
            }
            times.sort_unstable();
            eprintln!(
                "LOOP fuel={fuel} rep={rep} mean_ns={} p99_ns={} used={used}",
                times.iter().sum::<u128>() / 100,
                times[98]
            );
        }
    }
    Ok(())
}

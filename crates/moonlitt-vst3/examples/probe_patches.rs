//! Probe a VST3 plug-in's patch-switching surface: IUnitInfo program
//! lists and patch/browse/program-flavoured parameters. Read-only.
//!
//! Usage: cargo run -p moonlitt-vst3 --example probe_patches -- /path/to/Plugin.vst3

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: probe_patches <plugin.vst3>");
    let host = moonlitt_vst3::Vst3Host::new(48_000, 512).expect("host");
    let plugin = host
        .load_from_path(std::path::Path::new(&path))
        .expect("load plugin");

    println!("== IUnitInfo program lists ==");
    match plugin.presets() {
        Ok(presets) => {
            println!("presets: {}", presets.len());
            for p in presets.iter().take(10) {
                println!("  [list {} #{}] {}", p.list_id, p.program_index, p.name);
            }
        }
        Err(e) => println!("presets() -> {e:?}"),
    }

    println!("== parameters ==");
    let count = plugin.param_count();
    println!("parameter count: {count}");
    for i in 0..count {
        let Some(info) = plugin.param_info(i) else {
            continue;
        };
        let lower = info.name.to_lowercase();
        let interesting = info.is_program_change
            || lower.contains("brows")
            || lower.contains("patch")
            || lower.contains("program")
            || lower.contains("preset");
        if interesting || count <= 40 {
            println!(
                "  id={} name={:?} steps={} hidden={} readonly={}{}",
                info.id,
                info.name,
                info.step_count,
                info.is_hidden,
                info.is_readonly,
                if info.is_program_change {
                    " [PROGRAM-CHANGE]"
                } else {
                    ""
                },
            );
        }
    }
}

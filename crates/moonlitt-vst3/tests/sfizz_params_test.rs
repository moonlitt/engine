use moonlitt_vst3::Vst3Host;

#[test]
fn test_sfizz_params() {
    let host = Vst3Host::new(44100, 256).unwrap();
    let plugins = host.scan().unwrap();
    let sfizz = plugins.iter().find(|p| p.name == "sfizz");
    if sfizz.is_none() {
        eprintln!("sfizz not installed, skipping");
        return;
    }

    let plugin = host.load(sfizz.unwrap()).unwrap();
    let count = plugin.param_count();
    eprintln!("sfizz has {} params:", count);
    for i in 0..count {
        if let Some(info) = plugin.param_info(i) {
            eprintln!(
                "  [{:3}] id={:<8} {:30} default={:.3} step={} hidden={} readonly={} program_change={}",
                i, info.id, info.name, info.default_normalized,
                info.step_count, info.is_hidden, info.is_readonly, info.is_program_change
            );
        }
    }
}

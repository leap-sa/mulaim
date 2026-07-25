mod calc;
mod machine;
mod resolve;
mod server;

use calc::{build_report, CheckReport, Machine};

const SIGNATURE: &str = "Mulaim · ملائم — hosted & operated by LEAP RD&O واثب · https://leap.sa";

fn print_help(bin: &str) {
    println!("mulaim {} — will that local LLM fit comfortably on your machine?", env!("CARGO_PKG_VERSION"));
    println!("{SIGNATURE}");
    println!();
    println!("Usage:");
    println!("  {bin} <model|size> [--total GB] [--os GB] [--docker GB] [--json]");
    println!("  {bin} serve [--host 0.0.0.0] [--port 8080]");
    println!();
    println!("Examples:");
    println!("  {bin} 12b");
    println!("  {bin} qwen3:14b");
    println!("  {bin} unsloth/Qwen3-14B-GGUF");
    println!("  {bin} 27b --total 64 --os 10 --docker 8");
    println!("  {bin} serve                # bilingual (English/Arabic) web app + JSON API");
    println!();
    println!("Lookup order: raw size, local Ollama, Hugging Face, then name parsing.");
    println!("Web API: GET /api/check?model=12b&total=64&os=10&docker=8");
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bin = args.first().map(String::as_str).unwrap_or("mulaim");

    if args.len() == 1 || args.iter().any(|a| a == "--help" || a == "-h") {
        print_help(bin);
        return;
    }

    let mut machine = Machine::default();
    let mut total_overridden = false;
    let mut model_arg: Option<String> = None;
    let mut serve_mode = false;
    let mut json_out = false;
    let mut host = "0.0.0.0".to_string();
    let mut port: Option<u16> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "serve" if model_arg.is_none() && !serve_mode => serve_mode = true,
            "--total" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|v| v.parse().ok()) {
                    machine.total_unified_gb = v;
                    total_overridden = true;
                }
            }
            "--os" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|v| v.parse().ok()) {
                    machine.os_reserved_gb = v;
                }
            }
            "--docker" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|v| v.parse().ok()) {
                    machine.docker_reserved_gb = v;
                }
            }
            "--json" => json_out = true,
            "--host" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    host = v.clone();
                }
            }
            "--port" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|v| v.parse().ok()) {
                    port = Some(v);
                }
            }
            value if !value.starts_with('-') && model_arg.is_none() => {
                model_arg = Some(value.to_string());
            }
            _ => {}
        }
        i += 1;
    }

    if serve_mode {
        let port = port
            .or_else(|| std::env::var("PORT").ok().and_then(|p| p.parse().ok()))
            .unwrap_or(8080);
        if let Err(e) = server::run(&host, port).await {
            eprintln!("server error: {e}");
            std::process::exit(1);
        }
        return;
    }

    let Some(model_input) = model_arg else {
        eprintln!("Missing model size or name, e.g. `{bin} 12b` — or `{bin} serve` for the web app.");
        std::process::exit(1);
    };

    if !total_overridden {
        if let Some(d) = machine::detect() {
            machine.total_unified_gb = d.total_unified_gb;
            eprintln!(
                "(auto-detected {:.1} GB total memory{}; override with --total)",
                d.total_unified_gb,
                d.chip.as_deref().map(|c| format!(" on {c}")).unwrap_or_default()
            );
        }
    }

    let resolution = resolve::resolve(&model_input);
    let Some(params_b) = resolution.params_b else {
        eprintln!("Could not infer parameter count from: {model_input}");
        eprintln!("Try a name that includes the size (Qwen3-14B) or pass it directly (14b).");
        std::process::exit(1);
    };

    let report = build_report(&model_input, &resolution, params_b, &machine);

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("report serializes")
        );
        return;
    }
    print_report(&report);
}

fn print_report(r: &CheckReport) {
    println!("Model check report");
    println!("==================");
    println!("Machine: unified memory, {:.1} GB total", r.machine.total_unified_gb);
    println!("Reserved for OS/background: {:.1} GB", r.machine.os_reserved_gb);
    println!("Reserved for Docker/services: {:.1} GB", r.machine.docker_reserved_gb);
    println!("Safe LLM budget: {:.1} GB", r.machine.llm_budget_gb);
    println!();
    println!("Model input: {}", r.input);
    println!("Resolved via: {}", r.source);
    println!("Resolved model: {}", r.resolved_name);
    println!("Parameter count: {:.1}B", r.params_b);
    println!("Resolution notes: {}", r.note_en);
    println!("Rule of thumb: {}", r.recommendation_en);
    match &r.best {
        Some(b) => println!(
            "Best fit: {} ({}) — safe runtime {:.1} GB of {:.1} GB budget",
            b.label,
            b.fit.en(),
            b.runtime_total_gb,
            r.machine.llm_budget_gb
        ),
        None => println!("Best fit: none — does not fit safely; use a smaller model or lower quant."),
    }
    println!();
    println!("Quantization estimates");
    println!("----------------------");
    for q in &r.quants {
        println!(
            "{:<7} -> weights: {:.1} GB, min total (~1.1x): {:.1} GB, safe runtime (~1.5x): {:.1} GB, base fit: {}",
            q.label, q.weights_gb, q.formula_total_gb, q.runtime_total_gb, q.base_fit.en()
        );
    }
    println!();
    println!("Side workload fit");
    println!("-----------------");
    for wl in &r.workloads {
        println!("{}", wl.name_en);
        println!("  extra reserve: {:.1} GB", wl.reserve_gb);
        println!("  remaining LLM budget: {:.1} GB", wl.remaining_budget_gb);
        for f in &wl.fits {
            println!("  {:<7} => {}", f.quant, f.fit.en());
        }
        println!();
    }
    println!("Interpretation");
    println!("--------------");
    println!("Easy      = comfortable with headroom.");
    println!("Possible  = should run, but watch context length and concurrent apps.");
    println!("Tight     = likely to swap or feel sluggish; prefer lower quant or smaller model.");
    println!();
    println!("Tip: `{}` serve — bilingual (English/العربية) web version with the same engine.", "mulaim");
    println!("{SIGNATURE}");
}

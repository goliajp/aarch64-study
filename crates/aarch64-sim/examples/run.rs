//! Embed `aarch64-sim` in a plain Rust program.
//!
//! Boots the demo, runs for 4000 steps, and prints a few interesting fields.
//! Run with: `cargo run --example run -p aarch64-sim` (no features needed).

use aarch64_sim::Cpu;

fn main() {
    let mut cpu = Cpu::new();

    // Override the disk content so task B prints something custom.
    cpu.set_disk_text("hello, aarch64-sim!\n");

    let _retired = cpu.run(4000);

    println!("system steps: {}", cpu.system_steps());
    println!("timer ticks:  {}", cpu.timer_ticks());
    println!("atomic ctr:   {}", cpu.atomic_counter());

    let aic = cpu.aic_state();
    println!("AIC acks:     {}", aic.total_acks);
    println!("IPIs sent:    {}", aic.total_ipis);

    println!("UART output:  {:?}", cpu.output());

    for c in cpu.state() {
        println!(
            "core {}  {}  pc={:#010x}  el={}  wfi={}",
            c.id, c.kind, c.pc, c.current_el, c.wfi_halted,
        );
    }
}

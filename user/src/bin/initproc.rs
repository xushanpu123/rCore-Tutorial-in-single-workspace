#![no_std]
#![no_main]

extern crate user_lib;

use user_lib::{exec, fork, println, sched_yield, wait};

#[no_mangle]
fn main() -> i32 {
    println!("initproc!");
    let forkret = fork();
    if forkret == 0 {
        // exec("user_shell\0", &[core::ptr::null::<u8>()]);
        println!("exec");
        exec("user_shell\0");
    } else {
        loop {
            let mut exit_code: i32 = 0;
            let pid = wait(&mut exit_code);
            if pid == -1 {
                sched_yield();
                continue;
            }

            // println!(
            //     "[initproc] Released a zombie process, pid={}, exit_code={}",
            //     pid,
            //     exit_code,
            // );
        }
    }
    0
}

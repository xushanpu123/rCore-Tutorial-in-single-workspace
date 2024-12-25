#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

// not in SUCC_TESTS & FAIL_TESTS


// item of TESTS : app_name(argv_0), argv_1, argv_2, argv_3, exit_code
static SUCC_TESTS: &[(&str, &str, &str, &str, i32)] = &[
    ("00hello_world\0", "\0", "\0", "\0", 0),
    ("02power\0", "1\0", "2\0", "3\0", 0),
    ("12forktest\0", "\0", "\0", "\0", 0),
    ("13forktree\0", "\0", "\0", "\0", 0),
    ("14forktest2\0", "\0", "\0", "\0", 0),
    ("15matrix\0", "\0", "\0", "\0", 0),
    ("filetest_simple\0", "\0", "\0", "\0", 0),
    ("cat_filea\0", "\0", "\0", "\0", 0),
    // ("forktree\0", "\0", "\0", "\0", 0),
    ("sig_simple\0", "\0", "\0", "\0", 0),
    ("sig_simple2\0", "\0", "\0", "\0", 0),
    ("mpsc_sem\0", "\0", "\0", "\0", 0),
    ("sig_tests\0", "\0", "\0", "\0", 0),
    ("threads\0", "\0", "\0", "\0", 0),
    ("threads_arg\0", "\0", "\0", "\0", 0),
    ("sync_sem\0", "\0", "\0", "\0", 0),
    // ("pipetest\0", "\0", "\0", "\0", 0),
    // ("adder_peterson_spin\0", "\0", "\0", "\0", 0),
    // ("adder_peterson_yield\0", "\0", "\0", "\0", 0),
    // ("adder_mutex_blocking\0", "\0", "\0", "\0", 0),
    // ("adder_mutex_spin\0", "\0", "\0", "\0", 0),
    // ("run_pipe_test\0", "\0", "\0", "\0", 0),
    // ("sleep_simple\0", "\0", "\0", "\0", 0),
    // ("sleep\0", "\0", "\0", "\0", 0),
    // ("sleep_simple\0", "\0", "\0", "\0", 0),
    // ("sync_sem\0", "\0", "\0", "\0", 0),
    // ("condsync_sem\0", "\0", "\0", "\0", 0),
    // ("condsync_condvar\0", "\0", "\0", "\0", 0),
    // ("threads_arg\0", "\0", "\0", "\0", 0),
    // ("threads\0", "\0", "\0", "\0", 0),
    // ("yield\0", "\0", "\0", "\0", 0),
    // ("barrier_fail\0", "\0", "\0", "\0", 0),
    // ("barrier_condvar\0", "\0", "\0", "\0", 0),
];

static FAIL_TESTS: &[(&str, &str, &str, &str, i32)] = &[
    // ("03priv_inst\0", "\0", "\0", "\0", -11),
    ("01store_fault\0", "filea\0", "\0", "\0", 0),
    // ("04priv_csr\0", "\0", "\0", "\0", -6),
    // ("priv_csr\0", "\0", "\0", "\0", -4),
    // ("priv_inst\0", "\0", "\0", "\0", -4),
    // ("store_fault\0", "\0", "\0", "\0", -11),
    // ("until_timeout\0", "\0", "\0", "\0", -6),
    // ("adder\0", "\0", "\0", "\0", -6),
    // ("adder_simple_spin\0", "\0", "\0", "\0", -6),
    // ("adder_simple_yield\0", "\0", "\0", "\0", -6),
];

use user_lib::{exec, fork, waitpid};

fn run_tests(tests: &[(&str, &str, &str, &str, i32)]) -> i32 {
    let mut pass_num = 0;
    let mut arr: [*const u8; 4] = [
        core::ptr::null::<u8>(),
        core::ptr::null::<u8>(),
        core::ptr::null::<u8>(),
        core::ptr::null::<u8>(),
    ];

    for test in tests {
        println!("Usertests: Running {}", test.0);
        arr[0] = test.0.as_ptr();
        if test.1 != "\0" {
            arr[1] = test.1.as_ptr();
            arr[2] = core::ptr::null::<u8>();
            arr[3] = core::ptr::null::<u8>();
            if test.2 != "\0" {
                arr[2] = test.2.as_ptr();
                arr[3] = core::ptr::null::<u8>();
                if test.3 != "\0" {
                    arr[3] = test.3.as_ptr();
                } else {
                    arr[3] = core::ptr::null::<u8>();
                }
            } else {
                arr[2] = core::ptr::null::<u8>();
                arr[3] = core::ptr::null::<u8>();
            }
        } else {
            arr[1] = core::ptr::null::<u8>();
            arr[2] = core::ptr::null::<u8>();
            arr[3] = core::ptr::null::<u8>();
        }

        let pid = fork();
        if pid == 0 {
            exec(test.0);
            panic!("unreachable!");
        } else {
            let mut exit_code: i32 = Default::default();
            let wait_pid = waitpid(pid as isize, &mut exit_code);
            assert_eq!(pid, wait_pid);
            if exit_code == test.4 {
                // summary apps with  exit_code
                pass_num = pass_num + 1;
            }
            println!(
                "\x1b[32mUsertests: Test {} in Process {} exited with code {}\x1b[0m",
                test.0, pid, exit_code
            );
        }
    }
    pass_num
}

#[no_mangle]
pub fn main() -> i32 {
    let succ_num = run_tests(SUCC_TESTS);
    let err_num = run_tests(FAIL_TESTS);
    if succ_num == SUCC_TESTS.len() as i32 && err_num == 0 as i32 {
        println!(
            "{} of sueecssed apps, {} of failed apps run correctly. \nUsertests passed!",
            SUCC_TESTS.len(),
            FAIL_TESTS.len()
        );
        return 0;
    }
    if succ_num != SUCC_TESTS.len() as i32 {
        println!(
            "all successed app_num is  {} , but only  passed {}",
            SUCC_TESTS.len(),
            succ_num
        );
    }
    if err_num > 0 as i32 {
        println!(
            "all failed app_num is  {} , but only  passed {}",
            FAIL_TESTS.len(),
            FAIL_TESTS.len() as i32 - err_num
        );
    }
    println!(" Usertests failed!");
    return -1;
}

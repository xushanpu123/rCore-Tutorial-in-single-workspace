#!/usr/bin/env python3

import argparse
import os
from re import sub
import toml
import subprocess
from termcolor import colored
from liquid import Template, template

# Global Variables
target = "riscv64gc-unknown-none-elf"
elfPath = ""  # ELF Path, will be initialize when starting
binPath = ""  # Bin Path, will be initialize when starting
# Compile Mode , defualt debug. choices=["debug", "release"]
# Using --release to change it.
compileMode = "debug"

chapter = 1


# Get the target string of the target architecture
#
def getTarget():
    return target


def getTargetPath():
    return os.path.abspath("target/%s/%s" % (getTarget(), compileMode))
    # return "target/%s/%s" % (getTarget(), compileMode)


def parseArgs():
    parser = argparse.ArgumentParser(
        description="Run rCore tutorial in the qemu",
        epilog="Use this if you think xtask is difficult",
    )

    # Global Aruguments
    parser.add_argument(
        "--arch",
        "--architecture",
        choices=["riscv64", "aarch64", "x86_64", "loongarch64"],
        required=True,
    )
    parser.add_argument("--ch", "--chatper", type=int)
    parser.add_argument("--release", action="store_true")

    subparsers = parser.add_subparsers(help="sub command to run the kernel.")
    # Build Parser to parser arguments for building
    buildParser = subparsers.add_parser(
        "build", help="Build the source code to binary and elf file."
    )
    buildParser.set_defaults(func=build)
    buildParser.add_argument(
        "-log",
        choices=["trace", "debug", "info", "warn", "error"],
        default="info",
        help="Set the log level for the kernel",
    )
    # qemu Parser to parse arguments for running
    qemuParser = subparsers.add_parser(
        "qemu", help="Run the kernel in the qemu. rely on the build command"
    )
    qemuParser.set_defaults(func=qemu)

    # packfs Parser to parse arguments for pack easyfs
    packfsParser = subparsers.add_parser(
        "packfs", help="Build the easyfs with user tests files."
    )
    packfsParser.set_defaults(func=packfs)
    return parser.parse_args()


# Build Program for users
def buildUser(args):
    # Load config from the specific toml file.
    # Becase this toml file has a charactor '\ufeff', so using UTF-8-sig encoding to decode.
    casesList = toml.loads(open("user/cases-%s.toml" % (args.arch), "r+", encoding="UTF-8-sig").read())
    # Find the proper chatper
    chapterStr = "ch" + str(chapter)

    userConfig = casesList[chapterStr]
    print(userConfig)

    step = 0
    if "step" in userConfig:
        step = userConfig['step']

    for idx, case in enumerate(userConfig["cases"]):
        print(
            "    %s %s ..." % (colored("Building", "light_green", attrs=["bold"]), case)
        )
        command = [
            "cargo",
            "build",
            "--package",
            "user_lib",
            "--target",
            getTarget(),
            "--bin",
            case,
        ]
        env = os.environ
        if "base" in userConfig:
            env["base"] = str(userConfig["base"])
            env["BASE_ADDRESS"] = str(userConfig["base"] + (idx*step))
        if args.release:
            command += ["--release"]
        if args.arch == "loongarch64":
            command += ["-Zbuild-std=core,alloc"]
        subprocess.run(command)
        appElfPath = getTargetPath() + "/" + case
        if chapter <= 3:
            objcopy(appElfPath, appElfPath + ".bin")
        # else:
        #     objcopy(appElfPath, appElfPath, False)

    appAsmTemplate = Template("""
        .global apps
        .section .data
        .p2align 3
    apps:
        .quad {{ base }}
        .quad {{ step }}
        .quad {{ apps.size }}
        {% for app in apps %}

        .quad app_{{ forloop.index0 }}_start
        {% endfor %}
        .quad app_{{ apps.size | minus: 1 }}_end
        {% for app in apps %}
        app_{{ forloop.index0 }}_start:
        .incbin "{{ targetPath }}/{{ app }}{{ ext }}"
        app_{{ forloop.index0 }}_end:
        {% endfor %}

        {% if chapter == 5 %}      
        .p2align 3
        .section .data
        .global app_names
    app_names:
        {% for app in apps %}
        .string "{{ app }}"
        {% endfor %}
        {% endif %}
    """)
    base = 0
    step = 0
    ext  = ""
    if chapter <= 3:
        ext = ".bin"
    if "base" in userConfig:
        base = userConfig["base"]
    if "step" in userConfig:
        step = userConfig["step"]
    appAsm = appAsmTemplate.render(
        {
            "base": base,
            "step": step,
            "apps": userConfig["cases"],
            "targetPath": getTargetPath(),
            "ext": ext,
            "chapter": chapter
        }
    )
    with open(getTargetPath() + "/app.asm", "w+") as fp:
        fp.write(appAsm)


# Convert elf to binary, stripe all.
def objcopy(elfPath, binPath, binary=True):
    command = ["rust-objcopy", elfPath, "--strip-all", binPath]
    if binary:
        command += ["-O", "binary"]
    subprocess.run(command)


def packfs(args):
    buildUser(args)
    print("pack user fs")


def build(args):
    packfs(args)
    env = os.environ.copy()
    command = [
        "cargo",
        "build",
        "--target",
        getTarget(),
        "--package",
        "ch" + str(chapter),
    ]
    if chapter >= 2:
        env["APP_ASM"] = getTargetPath() + "/app.asm"
    if args.release:
        command += ["--release"]
    if args.arch == "loongarch64":
        command += ["-Zbuild-std=core,alloc"]
    res = subprocess.run(command, env=env)
    res.check_returncode()


# Run Kernel in the qemu
def qemu(args):
    build(args)
    command = ["qemu-system-" + args.arch, "-nographic"]

    objcopy(elfPath, binPath)
    if args.arch == "riscv64":
        command += ["--machine", "virt", "-kernel", binPath]
    elif args.arch == "aarch64":
        command += [
            "-cpu",
            "cortex-a72",
            "-machine",
            "virt",
            "-kernel",
            binPath,
        ]
        # @qemu-system-riscv64 -M 128m -machine virt,dumpdtb=virt.out
	    # fdtdump virt.out
    elif args.arch == "loongarch64":
        command += ["-kernel", elfPath]
    elif args.arch == "x86_64":
        command += ["-machine", "q35", "-kernel", elfPath, "-cpu", "IvyBridge-v2"]
    else:
        print(
            colored(
                "There is currently no support for %s! Comming soon!" % (args.arch),
                "red",
                attrs=["bold"],
            )
        )
        exit(0)

    # 64M is too small, so use 1G instead.
    command += ["-smp", "1", "-m", "1G", "-serial", "mon:stdio"]
    # Add debug arguments to qemu command and record the asm in the file
    command += ["-D", "qemu.log", "-d", "in_asm,int,pcall,cpu_reset,guest_errors"]

    # command += ["-s", "-S"]
    
    if chapter > 5:
        command += [
            "-drive",
            "file=%s/fs.img,if=none,format=raw,id=x0" % (getTargetPath()),
            "-device",
            "virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0",
        ]
    print(command)
    subprocess.run(command)


if __name__ == "__main__":
    args = parseArgs()
    # initialize Enviroment and Variable
    if args.release:
        compileMode = "release"
    target_list = {
        "riscv64": "riscv64gc-unknown-none-elf",
        "x86_64": "x86_64-unknown-none",
        "aarch64": "aarch64-unknown-none-softfloat",
        "loongarch64": "loongarch64-unknown-none",
    }
    target = target_list[args.arch]
    elfPath = "%s/ch%s" % (getTargetPath(), args.ch)
    binPath = elfPath + ".bin"
    chapter = args.ch

    # Display args and call specific function
    # build packfs or qemu
    # TODO: Call build and packfs if run qemu and pass argument -b(Build before run)
    print(args)
    args.func(args)

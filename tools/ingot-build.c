/* Ingot sudo wrapper.
 *
 * This workstation runs sudo-rs (Rust sudo), which cannot match sudoers
 * commands that are text scripts. The mkosi build needs root here (the
 * unprivileged user-namespace sandbox is blocked by the kernel LSM), so
 * this small ELF binary is the sudoers target. It execs
 * <REPO_ROOT>/tools/build.sh as root, passing all arguments through.
 *
 * Build:  cc -O2 -DREPO_ROOT='\"<repo>\"' -o tools/ingot-build tools/ingot-build.c
 * Sudoers (one command per line):
 *   <user> ALL=(root) NOPASSWD: /usr/local/bin/ingot-build
 *   <user> ALL=(root) NOPASSWD: /usr/bin/systemd-repart
 */
#ifndef REPO_ROOT
#define REPO_ROOT "/home/tommy/Ingot"
#endif

#include <stdio.h>
#include <unistd.h>

int main(int argc, char **argv)
{
    char *args[64];
    int i = 0;
    args[i++] = (char *)"bash";
    args[i++] = (char *)(REPO_ROOT "/tools/build.sh");
    for (int a = 1; a < argc && i < 62; a++)
        args[i++] = argv[a];
    args[i] = NULL;

    execvp("bash", args);
    perror("execvp bash");
    return 127;
}

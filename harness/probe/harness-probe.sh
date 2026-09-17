#!/bin/sh
# Harness boot probe: collects boot state into a JSON document, writes
# it to /var/lib/ingot-probe/harness-probe.json, and prints it between
# frame markers on the console (the QEMU harness parses it).
#
# Runs under /bin/sh (brush). POSIX-only: no awk/sed/grep/eval.
set -u

out=/var/lib/ingot-probe/harness-probe.json
mkdir -p /var/lib/ingot-probe

# Escape a value as a JSON string body (no surrounding quotes).
# Multi-line values are joined with literal \n.
je_tab=$(printf '\t')
je_cr=$(printf '\r')
jesc() {
    je_out=""
    je_first=1
    je_s=$1
    while :; do
        case $je_s in
            *"
"*)
                je_line=${je_s%%"
"*}
                je_s=${je_s#*"
"}
                je_last=0
                ;;
            *)
                je_line=$je_s
                je_s=""
                je_last=1
                ;;
        esac
        je_esc=""
        je_t=$je_line
        while [ -n "$je_t" ]; do
            je_c=${je_t%"${je_t#?}"}
            je_t=${je_t#?}
            case $je_c in
                \\) je_esc="$je_esc\\\\" ;;
                '"') je_esc="$je_esc\\\"" ;;
                "$je_tab") je_esc="$je_esc\\t" ;;
                "$je_cr") je_esc="$je_esc\\r" ;;
                *) je_esc="$je_esc$je_c" ;;
            esac
        done
        if [ "$je_first" = 1 ]; then
            je_out=$je_esc
            je_first=0
        else
            je_out="$je_out\\n$je_esc"
        fi
        [ "$je_last" = 1 ] && break
    done
    printf '%s' "$je_out"
}
read_mount() { # $1=mountpoint
    rm_opts=""; rm_fstype=""; rm_src=""
    while read -r rm_mid rm_par rm_dev rm_root rm_mpt rm_opts rm_rest; do
        [ "$rm_mpt" = "$1" ] || continue
        # rm_rest = [optional fields] "-" fstype source [super-options]
        set -- $rm_rest
        while [ "$#" -gt 3 ]; do
            [ "$1" = "-" ] && break
            shift
        done
        rm_fstype=$2
        rm_src=$3
        break
    done < /proc/self/mountinfo
}

kernel=$(uname -r)
pid1_comm=$(cat /proc/1/comm)
pid1_exe=$(readlink -f /proc/1/exe 2>/dev/null || echo unknown)
cmdline=$(tr '\0' ' ' < /proc/cmdline)
# booted slot's OS version: the slot-baked os-release (the UKI's .osrel
# is generated from the same content); identifies which slot booted.
version=""
if [ -r /usr/lib/os-release ]; then
    while IFS='=' read -r k v; do
        [ "$k" = VERSION_ID ] && { version=${v#\"}; version=${version%\"}; break; }
    done < /usr/lib/os-release
fi
# current CPU microcode revision (post early-load, /sys interface)
microcode_rev=$(cat /sys/devices/system/cpu/microcode/revision 2>/dev/null || echo unknown)

[ -e /sys/firmware/efi/sbs ] && secure_boot=1 || secure_boot=0
loader_features=$(cat /sys/firmware/efi/loader-features 2>/dev/null || true)
# Secure Boot diagnostics: what the guest actually exposes.
efivars_sb=0
[ -e /sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c ] && efivars_sb=1
sysfs_efi=$(ls /sys/firmware/efi 2>/dev/null | tr '\n' ' ')

read_mount /
root_fstype=$rm_fstype

read_mount /usr
[ -n "$rm_src" ] || { echo "probe: /usr is not a mount" >&2; exit 1; }
usr_src=$rm_src
usr_fstype=$rm_fstype
usr_opts=$rm_opts
# The mount source is the resolved block device (/dev/vdaN), so match
# it against /dev/disk/by-partuuid to recover the slot's GPT
# PARTUUID.
usr_resolved=$(readlink -f "$usr_src" 2>/dev/null || true)
usr_partuuid=""
for p in /dev/disk/by-partuuid/*; do
    [ -L "$p" ] || continue
    [ "$(readlink -f "$p" 2>/dev/null)" = "$usr_resolved" ] || continue
    usr_partuuid=$(basename "$p" | tr 'A-Z' 'a-z')
    break
done

read_mount /var
var_fstype=$rm_fstype
var_src=$rm_src

read_mount /etc
etc_is_mount=0
[ -n "$rm_src" ] && etc_is_mount=1
etc_src=$rm_src

[ -s /etc/fstab ] && fstab_nonempty=1 || fstab_nonempty=0

multi_user=$(systemctl show -p ActiveState --value multi-user.target 2>/dev/null || echo unknown)
system_running=$(systemctl is-system-running 2>/dev/null || true)

# Unit names of failed units, one per line. systemctl --failed prefixes
# each line with a load-state indicator (a bullet), so the unit name is
# the first whitespace-separated field that contains a dot (unit
# identifiers always do: foo.service, bar.socket, ...).
failed_units=""
fu_first=1
fu_raw=$(systemctl --failed --no-legend 2>/dev/null)
while IFS= read -r fu_line; do
    [ -z "$fu_line" ] && continue
    for fu_tok in $fu_line; do
        case $fu_tok in
            *.*)
                if [ $fu_first -eq 1 ]; then
                    failed_units=$fu_tok
                    fu_first=0
                else
                    failed_units="$failed_units
$fu_tok"
                fi
                break
                ;;
        esac
    done
done <<EOF
$fu_raw
EOF
journal_err=$(journalctl -b -p crit -q --no-pager 2>/dev/null || true)

sh_path=$(readlink -f /bin/sh 2>/dev/null || echo unknown)
# T3: the update helper is present in the slot and executable
helper_ok=0
/usr/bin/ingot-update-helper --help >/dev/null 2>&1 && helper_ok=1

doc=$(printf '{
  "kernel": "%s",
  "version": "%s",
  "pid1_comm": "%s",
  "pid1_exe": "%s",
  "cmdline": "%s",
  "secure_boot": %s,
  "efivars_sb": %s,
  "sysfs_efi": "%s",
  "loader_features": "%s",
  "root": {"fstype": "%s"},
  "usr": {"source": "%s", "fstype": "%s", "options": "%s", "partuuid": "%s"},
  "var": {"fstype": "%s", "source": "%s"},
  "etc": {"is_mount": %s, "source": "%s", "fstab_nonempty": %s},
  "multi_user": "%s",
  "system_running": "%s",
  "failed_units": "%s",
  "journal_err": "%s",
  "sh": "%s",
  "helper_ok": %s,
  "microcode_rev": "%s"
}' \
    "$(jesc "$kernel")" \
    "$(jesc "$version")" \
    "$(jesc "$pid1_comm")" \
    "$(jesc "$pid1_exe")" \
    "$(jesc "$cmdline")" \
    "$secure_boot" \
    "$efivars_sb" \
    "$(jesc "$sysfs_efi")" \
    "$(jesc "$loader_features")" \
    "$(jesc "$root_fstype")" \
    "$(jesc "$usr_src")" \
    "$(jesc "$usr_fstype")" \
    "$(jesc "$usr_opts")" \
    "$(jesc "$usr_partuuid")" \
    "$(jesc "$var_fstype")" \
    "$(jesc "$var_src")" \
    "$etc_is_mount" \
    "$(jesc "$etc_src")" \
    "$fstab_nonempty" \
    "$(jesc "$multi_user")" \
    "$(jesc "$system_running")" \
    "$(jesc "$failed_units")" \
    "$(jesc "$journal_err")" \
    "$(jesc "$sh_path")" \
    "$helper_ok" \
    "$(jesc "$microcode_rev")")

printf '%s\n' "$doc" > "$out"
# one write: keeps the frame as a single console record
frame=$(printf '%s\n%s\n%s' "=== INGOT-HARNESS-PROBE-BEGIN ===" "$doc" "=== INGOT-HARNESS-PROBE-END ===")
printf '%s\n' "$frame"

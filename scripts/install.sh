#!/usr/bin/env bash
# ============================================================
# ipaTool Management Script
#
# One-click install & management panel:
#   bash <(curl -fsSL https://cdn.jsdelivr.net/gh/ruanrrn/ipaTool@main/scripts/install.sh)
#
# After installation:
#   sudo bash /opt/ipatool/manager.sh
# ============================================================
set -euo pipefail

REPO="ruanrrn/ipaTool"
APP_NAME="ipatool"
INSTALL_DIR="/opt/${APP_NAME}"
SERVICE_NAME="ipatool"
GITHUB_API="https://api.github.com/repos/${REPO}"
CDN_BASE="https://cdn.jsdelivr.net/gh/${REPO}@main/scripts/install.sh"

# ─── Colors ──────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# ─── Logging ─────────────────────────────────────────────────
log_info()  { echo -e "${GREEN}[✓]${NC} $*"; }
log_warn()  { echo -e "${YELLOW}[!]${NC} $*"; }
log_error() { echo -e "${RED}[✗]${NC} $*"; }
log_step()  { echo -e "${CYAN}[→]${NC} $*"; }

# ─── Utilities ───────────────────────────────────────────────
require_root() {
    if [ "$(id -u)" -ne 0 ]; then
        log_error "This action requires root privileges. Run with sudo."
        return 1
    fi
}

press_enter() {
    echo ""
    echo -n "Press Enter to continue..."
    read -r
}

detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64) echo "amd64" ;;
        aarch64|arm64) echo "arm64" ;;
        *) echo "unsupported" ;;
    esac
}

get_latest_version() {
    curl -fsSL "${GITHUB_API}/releases/latest" 2>/dev/null \
        | grep -o '"tag_name": *"[^"]*"' | head -1 \
        | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/' \
        | sed 's/^v//'
}

# ─── Status Functions ────────────────────────────────────────

is_installed() { [ -f "${INSTALL_DIR}/server" ]; }

is_running() { systemctl is-active --quiet "${SERVICE_NAME}" 2>/dev/null; }

get_installed_version() {
    [ -f "${INSTALL_DIR}/.version" ] && cat "${INSTALL_DIR}/.version" || echo "unknown"
}

get_pid() {
    systemctl show "${SERVICE_NAME}" -p MainPID --value 2>/dev/null || echo "0"
}

get_memory() {
    local pid
    pid=$(get_pid)
    if [ "$pid" -gt 0 ] 2>/dev/null; then
        local mem_kb
        mem_kb=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ' || echo "0")
        if [ "${mem_kb:-0}" -gt 0 ] 2>/dev/null; then
            echo "$((mem_kb / 1024)) MB"
            return
        fi
    fi
    echo "N/A"
}

get_uptime() {
    if is_running; then
        local ts now_epoch ts_epoch diff
        ts=$(systemctl show "${SERVICE_NAME}" -p ActiveEnterTimestamp --value 2>/dev/null || echo "")
        if [ -n "$ts" ]; then
            now_epoch=$(date +%s)
            ts_epoch=$(date -d "$ts" +%s 2>/dev/null || echo "0")
            diff=$((now_epoch - ts_epoch))
            if [ "$diff" -gt 0 ]; then
                local d=$((diff / 86400))
                local h=$(((diff % 86400) / 3600))
                local m=$(((diff % 3600) / 60))
                [ "$d" -gt 0 ] && echo "${d}d ${h}h ${m}m" && return
                [ "$h" -gt 0 ] && echo "${h}h ${m}m" && return
                echo "${m}m"
                return
            fi
        fi
    fi
    echo "N/A"
}

get_port() {
    systemctl show "${SERVICE_NAME}" -p Environment 2>/dev/null \
        | grep -o 'PORT=[0-9]*' | cut -d= -f2 || echo "8080"
}

get_ip() {
    hostname -I 2>/dev/null | awk '{print $1}' || echo "127.0.0.1"
}

get_url() { echo "http://$(get_ip):$(get_port)"; }

get_latest_cached() {
    local cache_file="${INSTALL_DIR}/.latest_check"
    if [ -f "$cache_file" ]; then
        local cache_time now
        cache_time=$(stat -c %Y "$cache_file" 2>/dev/null || echo "0")
        now=$(date +%s)
        if [ $((now - cache_time)) -lt 3600 ]; then
            cat "$cache_file"
            return
        fi
    fi
    local latest
    latest=$(get_latest_version 2>/dev/null || echo "unknown")
    [ -d "$INSTALL_DIR" ] && echo "$latest" > "$cache_file" 2>/dev/null || true
    echo "$latest"
}

# ─── Actions ─────────────────────────────────────────────────

do_install() {
    require_root || { press_enter; return; }

    local was_installed=false
    [ -f "${INSTALL_DIR}/server" ] && was_installed=true

    local version="${1:-$(get_latest_version)}"
    local arch pkg_name dl_url
    arch=$(detect_arch)

    if [ "$arch" = "unsupported" ]; then
        log_error "Unsupported architecture: $(uname -m)"
        press_enter
        return
    fi

    pkg_name="ipatool-${version}-linux-${arch}.tar.gz"
    dl_url="https://github.com/${REPO}/releases/download/v${version}/${pkg_name}"

    echo ""
    log_step "Installing ipaTool v${version} (${arch})..."
    log_info "Downloading from GitHub Releases..."

    if ! curl -fsSL# -o "/tmp/${pkg_name}" "${dl_url}"; then
        log_error "Download failed. Does release v${version} exist for ${arch}?"
        press_enter
        return
    fi

    # Stop existing service
    if systemctl is-active --quiet "${SERVICE_NAME}" 2>/dev/null; then
        log_step "Stopping existing service..."
        systemctl stop "${SERVICE_NAME}" || true
    fi

    # Extract
    mkdir -p "${INSTALL_DIR}"
    tar xzf "/tmp/${pkg_name}" -C /tmp/
    local extracted_dir
    extracted_dir="/tmp/ipatool-${version}-linux-${arch}"

    if [ ! -d "$extracted_dir" ]; then
        extracted_dir=$(find /tmp -maxdepth 1 -type d -name "ipatool-${version}-*" 2>/dev/null | head -1)
    fi

    if [ ! -d "$extracted_dir" ]; then
        log_error "Extraction failed."
        rm -f "/tmp/${pkg_name}"
        press_enter
        return
    fi

    # Copy files
    cp -f "${extracted_dir}/server" "${INSTALL_DIR}/server"
    chmod +x "${INSTALL_DIR}/server"
    [ -d "${INSTALL_DIR}/dist" ] && rm -rf "${INSTALL_DIR}/dist"
    cp -r "${extracted_dir}/dist" "${INSTALL_DIR}/dist"

    # Cleanup
    rm -rf "$extracted_dir" "/tmp/${pkg_name}"

    # Save version
    echo "${version}" > "${INSTALL_DIR}/.version"

    # Generate admin password only on fresh install
    local admin_password=""
    if $was_installed; then
        log_info "Upgrade detected — preserving existing data."
    else
        admin_password=$(tr -dc 'A-Za-z0-9!@#^&*' < /dev/urandom | head -c 16)  # no '%': systemd unit specifier
        echo "${admin_password}" > "${INSTALL_DIR}/.initial_password"
        chmod 600 "${INSTALL_DIR}/.initial_password"
    fi

    # Setup systemd service
    if $was_installed; then
        systemctl daemon-reload
    else
        cat > "/etc/systemd/system/${SERVICE_NAME}.service" << SERVICEEOF
[Unit]
Description=ipaTool - IPA Download & Archive Manager
After=network.target

[Service]
Type=simple
ExecStart=${INSTALL_DIR}/server
WorkingDirectory=${INSTALL_DIR}
Restart=on-failure
RestartSec=5
Environment=IPA_ADMIN_INITIAL_PASSWORD=${admin_password}

[Install]
WantedBy=multi-user.target
SERVICEEOF
        systemctl daemon-reload
        systemctl enable "${SERVICE_NAME}" 2>/dev/null || true
    fi

    systemctl start "${SERVICE_NAME}" 2>/dev/null || true
    sleep 2

    # Save self for reuse — prefer copying from running instance to avoid CDN cache delay
    local self_dest="${INSTALL_DIR}/manager.sh"
    local saved=false

    # If running from a regular file, copy it directly (ensures identical version)
    if [ -f "$0" ] && [ "$0" != "$self_dest" ]; then
        if cp "$0" "$self_dest" 2>/dev/null; then
            chmod +x "$self_dest"
            saved=true
        fi
    fi

    # Otherwise download from GitHub raw (immediate updates), fall back to CDN
    if ! $saved; then
        if curl -fsSL "https://raw.githubusercontent.com/${REPO}/main/scripts/install.sh" -o "$self_dest" 2>/dev/null; then
            chmod +x "$self_dest"
        elif curl -fsSL "${CDN_BASE}" -o "$self_dest" 2>/dev/null; then
            chmod +x "$self_dest"
        else
            log_warn "Could not save management script locally."
        fi
    fi

    echo ""
    echo -e "${BOLD}${GREEN}╔══════════════════════════════════════════════════════════╗${NC}"
    echo -e "${BOLD}${GREEN}║              Installation Complete!                     ║${NC}"
    echo -e "${BOLD}${GREEN}╠══════════════════════════════════════════════════════════╣${NC}"
    if $was_installed; then
        printf "${BOLD}${GREEN}║${NC}  URL:      %-46s ${BOLD}${GREEN}║${NC}\n" "$(get_url)"
        printf "${BOLD}${GREEN}║${NC}  Version:  v%-43s ${BOLD}${GREEN}║${NC}\n" "${version}"
    else
        printf "${BOLD}${GREEN}║${NC}  URL:      %-46s ${BOLD}${GREEN}║${NC}\n" "$(get_url)"
        echo -e "${BOLD}${GREEN}║${NC}  Username: admin                                        ${BOLD}${GREEN}║${NC}"
        printf "${BOLD}${GREEN}║${NC}  Password: %-46s ${BOLD}${GREEN}║${NC}\n" "${admin_password}"
    fi
    echo -e "${BOLD}${GREEN}╠══════════════════════════════════════════════════════════╣${NC}"
    if $was_installed; then
        printf "${BOLD}${GREEN}║${NC}  Manage: sudo bash %-36s ${BOLD}${GREEN}║${NC}\n" "${INSTALL_DIR}/manager.sh"
    else
        echo -e "${BOLD}${GREEN}║${NC}${YELLOW}  Save this password! It will not be shown again.${NC}       ${BOLD}${GREEN}║${NC}"
        printf "${BOLD}${GREEN}║${NC}  Manage: sudo bash %-36s ${BOLD}${GREEN}║${NC}\n" "${INSTALL_DIR}/manager.sh"
    fi
    echo -e "${BOLD}${GREEN}╚══════════════════════════════════════════════════════════╝${NC}"
    echo ""

    press_enter
}

do_start() {
    require_root || { press_enter; return; }
    if is_running; then
        log_warn "Service is already running."
    else
        log_step "Starting service..."
        systemctl start "${SERVICE_NAME}"
        sleep 1
        is_running && log_info "Service started." \
            || log_error "Failed to start. Check: journalctl -u ${SERVICE_NAME} -n 20"
    fi
    press_enter
}

do_stop() {
    require_root || { press_enter; return; }
    if ! is_running; then
        log_warn "Service is already stopped."
    else
        log_step "Stopping service..."
        systemctl stop "${SERVICE_NAME}"
        log_info "Service stopped."
    fi
    press_enter
}

do_restart() {
    require_root || { press_enter; return; }
    log_step "Restarting service..."
    systemctl restart "${SERVICE_NAME}"
    sleep 1
    is_running && log_info "Service restarted." \
        || log_error "Failed to restart. Check: journalctl -u ${SERVICE_NAME} -n 20"
    press_enter
}

do_reset_password() {
    require_root || { press_enter; return; }

    local server_bin="${INSTALL_DIR}/server"
    if [ ! -f "$server_bin" ]; then
        log_error "Server not installed. Install ipaTool first."
        press_enter
        return
    fi

    echo ""
    log_warn "This invalidates all existing admin login sessions."
    echo ""

    local new_password
    new_password=$(tr -dc 'A-Za-z0-9!@#^&*' < /dev/urandom | head -c 16)

    log_step "Resetting admin password..."
    # IPA_TOOL_ROOT: the binary otherwise resolves data/ relative to the caller's working directory.
    if printf '%s' "$new_password" | IPA_TOOL_ROOT="${INSTALL_DIR}" "$server_bin" reset-admin-password --username admin --password-stdin; then
        echo "${new_password}" > "${INSTALL_DIR}/.initial_password"
        chmod 600 "${INSTALL_DIR}/.initial_password"
        echo ""
        echo -e "${BOLD}${GREEN}  New admin password: ${new_password}${NC}"
        echo ""
        log_warn "Save this password. It will not be shown again."
        log_info "You can also view it later with option [6] in the management panel."
    else
        log_error "Password reset failed. Is the server binary functional?"
    fi
    press_enter
}

do_view_initial_password() {
    if [ -f "${INSTALL_DIR}/.initial_password" ]; then
        echo ""
        echo -e "${BOLD}  Initial admin password: ${GREEN}$(cat ${INSTALL_DIR}/.initial_password)${NC}"
        echo ""
    else
        log_warn "No initial password record found."
        log_info "It may have been changed or the record was deleted."
        log_info "Use option [5] to reset the password."
    fi
    press_enter
}

do_change_port() {
    require_root || { press_enter; return; }

    local current_port new_port
    current_port=$(get_port)

    echo ""
    echo -n "Enter new port (current: ${current_port}): "
    read -r new_port

    if ! [[ "$new_port" =~ ^[0-9]+$ ]] || [ "$new_port" -lt 1 ] || [ "$new_port" -gt 65535 ]; then
        log_error "Invalid port: ${new_port}"
        press_enter
        return
    fi

    log_step "Changing port from ${current_port} to ${new_port}..."

    local service_file="/etc/systemd/system/${SERVICE_NAME}.service"
    if grep -q "Environment=PORT=" "$service_file" 2>/dev/null; then
        sed -i "s/Environment=PORT=[0-9]*/Environment=PORT=${new_port}/" "$service_file"
    else
        sed -i "/^\[Service\]/a Environment=PORT=${new_port}" "$service_file"
    fi

    systemctl daemon-reload
    systemctl restart "${SERVICE_NAME}"
    sleep 1

    if is_running; then
        log_info "Port changed. New URL: http://$(get_ip):${new_port}"
    else
        log_error "Service failed to start. Reverting port..."
        sed -i "s/Environment=PORT=[0-9]*/Environment=PORT=${current_port}/" "$service_file"
        systemctl daemon-reload
        systemctl restart "${SERVICE_NAME}"
    fi
    press_enter
}

do_view_logs() {
    echo ""
    echo -e "${BOLD}Recent logs (press q to exit):${NC}"
    echo ""
    if command -v journalctl &>/dev/null; then
        journalctl -u "${SERVICE_NAME}" -n 50 --no-pager 2>/dev/null || echo "No logs available."
    else
        log_warn "journalctl not available."
    fi
    echo ""
    press_enter
}

# ─── Panel ───────────────────────────────────────────────────

show_panel() {
    clear

    if is_installed; then
        local version status pid mem uptime port url
        version=$(get_installed_version)
        port=$(get_port)
        url=$(get_url)

        if is_running; then
            status="${GREEN}● Running${NC}"
            pid="(PID $(get_pid))"
            mem=$(get_memory)
            uptime=$(get_uptime)
        else
            status="${RED}● Stopped${NC}"
            pid=""
            mem="N/A"
            uptime="N/A"
        fi

        # Check for update
        local latest update_hint=""
        latest=$(get_latest_cached 2>/dev/null || echo "")
        if [ -n "$latest" ] && [ "$latest" != "unknown" ] && [ "$version" != "unknown" ] && [ "$latest" != "$version" ]; then
            update_hint="  ${YELLOW}(v${latest} available!)${NC}"
        fi

        echo ""
        echo -e "${BOLD}${BLUE}╔══════════════════════════════════════════════════════════╗${NC}"
        echo -e "${BOLD}${BLUE}║${NC}              ${BOLD}ipaTool Management Panel${NC}                    ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}╠══════════════════════════════════════════════════════════╣${NC}"
        local version_display="v${version}"
        if [ -n "$update_hint" ]; then
            printf "${BOLD}${BLUE}║${NC}  ${BOLD}Version:${NC}   %-10s %b ${BOLD}${BLUE}║${NC}\n" "${version_display}" "${update_hint}"
        else
            printf "${BOLD}${BLUE}║${NC}  ${BOLD}Version:${NC}   %-44s ${BOLD}${BLUE}║${NC}\n" "${version_display}"
        fi
        printf "${BOLD}${BLUE}║${NC}  ${BOLD}Status:${NC}    %b%-42s ${BOLD}${BLUE}║${NC}\n" "$status" "  ${pid}"
        printf "${BOLD}${BLUE}║${NC}  ${BOLD}Memory:${NC}    %-44s ${BOLD}${BLUE}║${NC}\n" "$mem"
        printf "${BOLD}${BLUE}║${NC}  ${BOLD}Uptime:${NC}    %-44s ${BOLD}${BLUE}║${NC}\n" "$uptime"
        printf "${BOLD}${BLUE}║${NC}  ${BOLD}Port:${NC}      %-44s ${BOLD}${BLUE}║${NC}\n" "$port"
        printf "${BOLD}${BLUE}║${NC}  ${BOLD}URL:${NC}       %-44s ${BOLD}${BLUE}║${NC}\n" "$url"
        echo -e "${BOLD}${BLUE}╠══════════════════════════════════════════════════════════╣${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}[1]${NC} Install / Update                                  ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}[2]${NC} Start                                             ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}[3]${NC} Stop                                              ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}[4]${NC} Restart                                           ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}[5]${NC} Reset Admin Password                              ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}[6]${NC} View Initial Password                             ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}[7]${NC} Change Port                                       ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}[8]${NC} View Logs                                         ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}[q]${NC} Quit                                              ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}╚══════════════════════════════════════════════════════════╝${NC}"
        echo ""
    else
        echo ""
        echo -e "${BOLD}${BLUE}╔══════════════════════════════════════════════════════════╗${NC}"
        echo -e "${BOLD}${BLUE}║${NC}              ${BOLD}ipaTool Management Panel${NC}                    ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}╠══════════════════════════════════════════════════════════╣${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}Status:${NC}    ${RED}● Not Installed${NC}                               ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}╠══════════════════════════════════════════════════════════╣${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}[1]${NC} Install ipaTool                                   ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}║${NC}  ${BOLD}[q]${NC} Quit                                              ${BOLD}${BLUE}║${NC}"
        echo -e "${BOLD}${BLUE}╚══════════════════════════════════════════════════════════╝${NC}"
        echo ""
    fi

    echo -n "Select option: "
    read -r choice
    # Strip carriage return from terminals that send CR+LF (e.g., PuTTY, serial consoles)
    choice="${choice%$'\r'}"

    case "$choice" in
        1)
            if is_installed; then
                local latest_ver
                latest_ver=$(get_latest_version 2>/dev/null || echo "")
                [ -n "$latest_ver" ] && [ "$latest_ver" != "unknown" ] \
                    && do_install "$latest_ver" \
                    || { log_error "Cannot fetch latest version. Check network."; press_enter; }
            else
                do_install
            fi
            ;;
        2) is_installed && do_start || { log_error "Install ipaTool first."; press_enter; } ;;
        3) is_installed && do_stop || { log_error "Install ipaTool first."; press_enter; } ;;
        4) is_installed && do_restart || { log_error "Install ipaTool first."; press_enter; } ;;
        5) is_installed && do_reset_password || { log_error "Install ipaTool first."; press_enter; } ;;
        6) is_installed && do_view_initial_password || { log_error "Install ipaTool first."; press_enter; } ;;
        7) is_installed && do_change_port || { log_error "Install ipaTool first."; press_enter; } ;;
        8) is_installed && do_view_logs || { log_error "Install ipaTool first."; press_enter; } ;;
        q|Q) echo ""; echo "Goodbye!"; exit 0 ;;
        *) echo "Invalid option."; press_enter ;;
    esac
}

# ─── Main ────────────────────────────────────────────────────

main() {
    if [ "$(uname -s)" != "Linux" ]; then
        log_error "This script only supports Linux."
        exit 1
    fi

    while true; do
        show_panel
    done
}

main "$@"

#!/bin/sh
# install.sh — Install tracegrep from GitHub releases
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/btucker/tracegrep/main/install.sh | sh
#
# Environment variables (set on the sh side of the pipe, not the curl side):
#   VERSION      — specific version to install (e.g., "v0.1.0"); defaults to latest
#   INSTALL_DIR  — installation directory; defaults to ~/.local/bin
#
# Example:
#   curl -fsSL https://raw.githubusercontent.com/btucker/tracegrep/main/install.sh | INSTALL_DIR=/usr/local/bin sh

set -eu

REPO="btucker/tracegrep"
BINARY_NAME="tracegrep"
DEFAULT_INSTALL_DIR="$HOME/.local/bin"

info() {
    printf '%s\n' "$@"
}

err() {
    printf 'error: %s\n' "$@" >&2
    exit 1
}

need_cmd() {
    if ! command -v "$1" > /dev/null 2>&1; then
        err "need '$1' (command not found)"
    fi
}

detect_platform() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os="unknown-linux-gnu" ;;
        Darwin) os="apple-darwin" ;;
        *)      err "unsupported OS: $os" ;;
    esac

    case "$arch" in
        x86_64|amd64)   arch="x86_64" ;;
        aarch64|arm64)  arch="aarch64" ;;
        *)              err "unsupported architecture: $arch" ;;
    esac

    TARGET="${arch}-${os}"
}

get_latest_version() {
    url="https://api.github.com/repos/${REPO}/releases/latest"
    if command -v curl > /dev/null 2>&1; then
        response=$(curl -fsSL "$url") || err "failed to fetch latest release info"
    elif command -v wget > /dev/null 2>&1; then
        response=$(wget -qO- "$url") || err "failed to fetch latest release info"
    else
        err "need 'curl' or 'wget' to download files"
    fi

    version=$(printf '%s' "$response" | grep '"tag_name"' | sed 's/.*"tag_name": *"//;s/".*//')
    if [ -z "$version" ]; then
        err "could not determine latest version"
    fi
    printf '%s' "$version"
}

download() {
    url="$1"
    dest="$2"
    if command -v curl > /dev/null 2>&1; then
        curl -fsSL -o "$dest" "$url"
    elif command -v wget > /dev/null 2>&1; then
        wget -qO "$dest" "$url"
    else
        err "need 'curl' or 'wget' to download files"
    fi
}

main() {
    need_cmd uname
    need_cmd tar

    detect_platform

    if [ -n "${VERSION:-}" ]; then
        version="$VERSION"
    else
        info "Fetching latest release..."
        version=$(get_latest_version)
    fi

    install_dir="${INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"

    info "Installing ${BINARY_NAME} ${version} (${TARGET})"
    info "  to: ${install_dir}"

    # Build download URL
    tarball="${BINARY_NAME}-${version}-${TARGET}.tar.gz"
    download_url="https://github.com/${REPO}/releases/download/${version}/${tarball}"

    # Create temp directory
    tmp_dir=$(mktemp -d)
    trap 'rm -rf "$tmp_dir"' EXIT

    # Download
    info "Downloading ${download_url}..."
    download "$download_url" "${tmp_dir}/${tarball}" || err "download failed — check that version '${version}' exists and has a release asset for ${TARGET}"

    # Extract
    tar -xzf "${tmp_dir}/${tarball}" -C "$tmp_dir" || err "failed to extract tarball"

    # Install
    mkdir -p "$install_dir"
    if [ -f "${tmp_dir}/${BINARY_NAME}" ]; then
        mv "${tmp_dir}/${BINARY_NAME}" "${install_dir}/${BINARY_NAME}"
    elif [ -f "${tmp_dir}/${BINARY_NAME}-${version}-${TARGET}/${BINARY_NAME}" ]; then
        mv "${tmp_dir}/${BINARY_NAME}-${version}-${TARGET}/${BINARY_NAME}" "${install_dir}/${BINARY_NAME}"
    else
        err "could not find '${BINARY_NAME}' binary in archive"
    fi
    chmod +x "${install_dir}/${BINARY_NAME}"

    info ""
    info "Successfully installed ${BINARY_NAME} to ${install_dir}/${BINARY_NAME}"

    # Check if install dir is on PATH
    case ":${PATH}:" in
        *":${install_dir}:"*) ;;
        *)
            info ""
            info "WARNING: ${install_dir} is not in your PATH."
            info "Add it with:"
            info "  export PATH=\"${install_dir}:\$PATH\""
            ;;
    esac

    # Check for ripgrep dependency
    if ! command -v rg > /dev/null 2>&1; then
        info ""
        info "WARNING: 'rg' (ripgrep) was not found on PATH."
        info "tracegrep requires ripgrep to function."
        info "Install it from: https://github.com/BurntSushi/ripgrep#installation"
    fi

    # Also install the 'tg' alias
    if [ -f "${tmp_dir}/tg" ]; then
        mv "${tmp_dir}/tg" "${install_dir}/tg"
        chmod +x "${install_dir}/tg"
    elif [ -f "${tmp_dir}/${BINARY_NAME}-${version}-${TARGET}/tg" ]; then
        mv "${tmp_dir}/${BINARY_NAME}-${version}-${TARGET}/tg" "${install_dir}/tg"
        chmod +x "${install_dir}/tg"
    fi
}

main

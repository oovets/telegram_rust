#!/bin/bash

# Build script för Telegram Client Rust

echo "🦀 Building Telegram Client (Rust)..."
echo ""

# Check if Rust is installed
if ! command -v cargo &> /dev/null; then
    echo "❌ Rust is not installed!"
    echo "Install from: https://rustup.rs/"
    exit 1
fi

echo "✅ Rust found: $(rustc --version)"
echo ""

# Build in release mode for maximum performance
echo "🔨 Compiling with optimizations..."
cargo build --release

if [ $? -eq 0 ]; then
    echo ""
    echo "✅ Build successful!"
    echo ""
    echo "📦 Binary location: target/release/telegram_client_rs"
    echo "💾 Size: $(du -h target/release/telegram_client_rs | cut -f1)"
    echo ""
    echo "🚀 Run with: cargo run --release"
    echo "   or:       ./target/release/telegram_client_rs"
else
    echo ""
    echo "❌ Build failed!"
    exit 1
fi

#!/data/data/com.termux/files/usr/bin/bash
# Cài Kinet (ngôn ngữ lập trình viết bằng Rust) trên Termux (Android).
#
# Cách dùng:  bash install_termux.sh
# Sau khi xong, gõ `kinet` là chạy được (kèm REPL).

set -e

echo "==> 1/4 Cài gói cần thiết (rust + git)"
pkg update -y
pkg install -y rust git

echo "==> 2/4 Lấy mã nguồn Kinet"
if [ -d "$HOME/Kinet" ]; then
  echo "Đã có $HOME/Kinet — cập nhật..."
  git -C "$HOME/Kinet" pull
else
  git clone https://github.com/maycockwollam-netizen/Kinet.git "$HOME/Kinet"
fi

echo "==> 3/4 Build bản release (mất vài phút, cần mạng để tải indexmap)"
cd "$HOME/Kinet"
cargo build --release

echo "==> 4/4 Đặt lệnh 'kinet' vào PATH"
cp target/release/kinet "$PREFIX/bin/kinet"

echo ""
echo "✅ Xong! Chạy thử:"
echo "   kinet examples/file_report.kinet     # công cụ mẫu"
echo "   kinet examples/demo.kinet            # demo đệ quy"
echo "   kinet                                # mở REPL"
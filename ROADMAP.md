# ROADMAP — Lịch trình phát triển Kinet thành ngôn ngữ thật

> Mục tiêu: từ ngôn ngữ nhỏ hiện tại → ngôn ngữ script **Sài được thật** để điều
> khiển Kinetic SDK của mình, rồi mới xa hơn.
>
> Ước lượng theo "ngày làm đều" (nửa ngày / buổi). Càng về sau càng mở — điều
> chỉnh theo tốc độ và nhu cầu.

## 🗺️ Vị trí đang đứng
| Mốc | Hoàn thành |
|---|---|
| M1 match, list, native | ✅ |
| M2 for/break/continue, string funcs | ✅ |
| M3 agent_run gọi SDK thật (bridge thô) | ✅ |
| M4 map, file I/O, đệ quy sâu, mini_interp | ✅ |

---

## 🟢 GIAI ĐOẠN A — "Dùng được cho SDK" (ưu tiên cao nhất)

Cái mày cần NGAY: viết agent script thật, gọi SDK liên tục, trao đổi dữ liệu có cấu trúc.

### A1 — Cú pháp sướng tay  (≈ 2–3 buổi)
- [ ] Dict literal `{ "a": 1, "b": 2 }` (giải quyết va chạm `{` với block)
- [ ] Template chuỗi `${x}` / `"{name} = {value}"`
- [ ] `for i in range(...)` tiện hơn (đã có), `for key in map_keys(m)`
- [ ] Comments khối `/* ... */`, dòng lệnh `;` không bắt buộc cuối block

### A2 — Bridge thật (có cấu trúc + giữ tiến trình)  (≈ 3–4 buổi)
- [ ] Giữ 1 tiến trình Python sống giữa các lần gọi (đọc/ghi qua stdin/stdout)
- [ ] `agent_run(config_map)` → trả `{output, tool_calls, duration, token_usage}`
- [ ] Native `agent_config(...)`, `agent_status()` kiểu metadata
- [ ] Mở rộng contact: subagent, memory, verify... (tùy cần)

### A3 — Lỗi + stdlib lite  (≈ 3 buổi)
- [ ] **Stack trace** khi script lỗi (dòng nào, hàm nào gọi)
- [ ] Bộ I/O đầy đủ: append_file, read_lines, list_dir, exists
- [ ] Helper list/map: map_values, list_join, string uppercase/lowercase/substring
- [ ] `def`/gọi với kiểm tra số đối số rõ hơn

### A4 — Script mẫu đời thực  (≈ 1–2 buổi)
- [ ] `examples/sdk_workflow.kinet`: đọc danh sách task từ file → chạy agent từng task →
      gom kết quả vào map → ghi báo cáo ra file
- [ ] Ví dụ retry + rẽ nhánh theo `tool_calls`

**Kết quả Giai đoạn A:** Kinet Sài được thật cho việc viết agent script của mình.
**Tổng: ~10–12 buổi (≈ 2–3 tuần đều tay).**

---

## 🟡 GIAI ĐOẠN B — "Ngôn ngữ đứng vững" (đủ làm phần mềm cỡ vừa)

### B1 — Module / import  (≈ 3–4 buổi)
- [ ] Chia code thành nhiều file, `import "file.kinet"`, namespace
- [ ] Thư mục stdlib riêng

### B2 — Kiểu + ngữ nghĩa  (≈ 4–5 buổi)
- [ ] `float`, ký tự riêng, `null` rõ ràng
- [ ] Hàm ẩn danh / closure dạng biểu thức (lambda) → map/filter
- [ ] Tuple / struct nhẹ (map có khoá cố định + validate)

### B3 — Xử lý lỗi  (≈ 3 buổi)
- [ ] `try` / `throw` / `catch` (kiểu ngoại lệ)
- [ ] `finally` / rã giải tài nguyên

### B4 — Test + tài liệu  (≈ 3 buổi)
- [ ] Framework test có sẵn trong ngôn ngữ (`assert`, `test "tên" { }`)
- [ ] Docs ngắn cho từng feature

**Tổng Giai đoạn B: ~16–20 buổi (≈ 1–1.5 tháng).**

---

## 🟠 GIAI ĐOẠN C — "Hiệu năng & công cụ" (chạy nhanh, xài mượt)

### C1 — Bytecode / VM  (≈ 1–2 tháng)
- [ ] Biên dịch AST → bytecode, VM loop thay vì tree-walking
- [ ] Đo benchmark, tối ưu hot path

### C2 — Embed PyO3  (≈ 1 tuần)
- [ ] Bỏ spawn tiến trình — gọi SDK qua embed trực tiếp (như `_kinetic_rs`)

### C3 — Tooling  (≈ 2–3 tuần)
- [ ] Package manager lite, linter, formatter, REPL nâng cao (history, tab)

**Tổng Giai đoạn C: ~2–3 tháng.**

---

## 🔴 GIAI ĐOẠN D — "Nghiêm túc / tự host" (dài hạn, mở)

- [ ] **Self-host**: viết lại interpreter bằng chính Kinet (mục tiêu tượng trưng)
- [ ] Hệ type tĩnh (annotation + suy luận)
- [ ] Compiler sang native/bytecode tối ưu
- [ ] GCC/stdlib đầy đủ, cộng đồng (nếu muốn)

**Tổng: 1 năm trở lên**, không có mốc cố định.

---

## 📌 Ghi chú chiến lược
- **Giai đoạn A là thứ bạn cần NGAY** — tập trung vào đó trước.
- **SDK dùng Kinet lúc nào?** Ngay sau A (chưa cần B/C). B/C nâng cấp dần.
- Đừng nhảy thẳng C (hiệu năng) trước khi A xong — học tốc độ, ưu tiên đúng.
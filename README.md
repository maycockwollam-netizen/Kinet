# Kinet — My Programming Language

Một **ngôn ngữ lập trình đa dụng** viết bằng **Rust**, tự xây dựng theo kiến trúc kinh điển.
Dùng để viết đủ loại chương trình — agent, công cụ, thư viện, thậm chí một trình thông
dịch/ngôn ngữ khác. Nói chung mày muốn tạo gì cũng được, miễn là viết được bằng code.

```
Code text → Lexer → Token → Parser → AST → Interpreter → Kết quả
```

## Chạy

```bash
cargo run                     # mở REPL
cargo run -- file.kinet         # chạy file source
cargo run -- -e "code"        # chạy code từ dòng lệnh
cargo run -- --tokens file    # xem danh sách token (Lexer)
cargo run -- --ast file       # xem cây cú pháp (Parser)
cargo test                    # chạy unit test
```

Ví dụ: `cargo run -- examples/demo.kinet`

## Cú pháp

```c
// comment
let x = 2 + 3 * 4;            // biến, số, toán tử số học

println("x = ", x);           // in nhiều giá trị (tự nối chuỗi)

if x > 10 and not false {     // if / else if / else
    println("lon");
} else if x == 14 {
    println("bang");
} else {
    println("be");
}

let i = 0;
while i < 3 {                 // vòng lặp while
    i = i + 1;                // gán lại biến
}

for n in range(0, 5) {        // vòng lặp for — lặp qua list/range
    if n == 3 { continue; }   // bỏ qua vòng này
    if n == 4 { break; }      // thoát hẳn
    println(n);
}

func add(a, b) {              // hàm + đệ quy
    return a + b;
}
println(add(2, 5));

try {                          // bắt lỗi runtime
    let r = risky();
} catch e {
    println("Thất bại: ", e);  // e = thông báo lỗi (kèm số dòng)
}

throw "hết tiền";              // chủ động ném lỗi (try/catch bắt được)

let a = [1, 2, 3];
a[1] = 99;                     // ghi mục trong list
let m = { "x": 1 };
m["y"] = 2;                    // ghi mục trong map
println(a[1], m["y"]);         // đọc mục — kể cả lồng: grid[r][c]
```

### Kiểu dữ liệu hiện có
| Kiểu | Ví dụ |
|---|---|
| Số nguyên `i64` | `42`, `-7` |
| Số thập phân `f64` | `3.14`, `-2.5` — trộn được với số nguyên |
| Chuỗi | `"xin chào"` (hỗ trợ `\n`, `\t`, `\"`, `\\`) |
| Bool | `true`, `false` |
| List | `[1, "hai", true]` — có thể trộn kiểu |
| Map | `{ "a": 1, "b": 2 }` hoặc `map("a", 1, ...)` — giữ thứ tự chèn |
| Hàm | `func f(x) { ... }` |
| Nil | giá trị trả về mặc định |

### Hàm dựng sẵn (native)
| Hàm | Ý nghĩa | Ví dụ |
|---|---|---|
| `len(x)` | độ dài chuỗi hoặc list | `len("chao")` → `4` |
| `get(list, i)` | lấy phần tử (0-based) | `get([10, 20], 1)` → `20` |
| `contains(list, x)` / `contains(s, sub)` | list/chuỗi có chứa? | `contains([1, 2], 2)` → `true` |
| `append(list, x)` | list mới + thêm phần tử (thuần, không đổi gốc) | `append([1], 2)` → `[1, 2]` |
| `range(start, end[, step])` | list số nguyên (không gồm end) | `range(0, 5)` → `[0,1,2,3,4]` |
| `split(s, sep)` | tách chuỗi → list | `split("a,b", ",")` → `["a","b"]` |
| `starts_with(s, p)` / `ends_with(s, p)` | kiểm tra tiền/hậu tố | `starts_with("run: x", "run:")` → `true` |
| `trim(s)` | bỏ khoảng trắng 2 đầu | `trim("  hi  ")` → `"hi"` |
| `replace(s, a, b)` | thay hết `a` bằng `b` | `replace("a-b", "-", "+")` → `"a+b"` |
| `map(k, v, ...)` | tạo map (số chẵn đối số) | `map("a", 1)` |
| `map_set(m, k, v)` | map mới, đặt khóa `k = v` | `map_set(m, "b", 2)` |
| `map_keys(m)` | list các khóa | `map_keys(m)` → `["a"]` |
| `map_remove(m, k)` | map mới, bỏ khóa `k` | `map_remove(m, "a")` |
| `get(m, k)` / `get(list, i)` | lấy giá trị theo khóa / chỉ số | `get(m, "a")` |
| `contains(m, k)` | map có khóa `k`? | `contains(m, "a")` → `true` |
| `read_file(p)` | đọc nội dung file | `read_file("a.txt")` |
| `write_file(p, s)` | ghi nội dung file | `write_file("a.txt", "hi")` |
| `chars(s)` | list các ký tự (chuỗi 1 chữ) | `chars("ab")` → `["a","b"]` |
| `ord(c)` | mã Unicode của ký tự đầu | `ord("0")` → `48` |
| `to_int(x)` | số/chuỗi → số nguyên | `to_int("42")` → `42`, `to_int(3.9)` → `3` |
| `to_float(x)` | số/chuỗi → số thập phân | `to_float("3.14")` → `3.14` |
| `str(x)` | mọi giá trị → chuỗi | `str({ "a": 1 })` → `"{a: 1}"` |
| `exists(p)` | đường dẫn có tồn tại? | `exists("a.txt")` → `true` |
| `append_file(p, s)` | ghi THÊM vào file | `append_file("log.txt", "x")` |
| `list_dir(p)` | liệt kê tên file (sắp xếp) | `list_dir(".")` |
| `read_lines(s)` | chuỗi → list dòng | `read_lines("a\nb")` → `["a","b"]` |
| `substring(s, start, len)` | cắt chuỗi | `substring("hello", 1, 3)` → `"ell"` |
| `to_upper(s)` / `to_lower(s)` | viết hoa / thường | `to_upper("ab")` → `"AB"` |
| `list_join(list, sep)` | nối list thành chuỗi | `list_join(["a","b"], "-")` → `"a-b"` |

### match — pattern dispatch (cho logic agent)
```c
let status = match result {
    case "success" => "done";
    case err       => "đang chạy (" + err + ")";   // bắt giá trị vào biến err
    case _         => "lạ quá";                     // wildcard
    default        => "mặc định";                   // wildcard dạng chữ
};
```
Pattern hỗ trợ: literal (`1`, `"x"`, `true`/`false`), biến bắt (`case e => ...`), wildcard (`_` / `default`). Nhánh đầu tiên khớp sẽ thắng — thứ tự quan trọng.

### Toán tử
- Số học: `+ - * / %` (có `+` cho chuỗi — nối kiểu)
- So sánh: `== != < > <= >=`
- Bit: `& | << >>` (chỉ số nguyên)
- Logic: `and` `or` `not` (đánh giá lười — short-circuit)
- Phép gán: `=` `a[i] = v` `m["k"] = v`

## Viết phần mềm khác bằng Kinet (Mốc 4)

Kinet đã đủ tính năng cho **lập trình đa dụng**: `map` (dictionary), file I/O, đệ quy sâu (độ sâu 2000), hàm chuỗi, list. Bằng chứng là `examples/mini_interp.kinet` — **một trình thông dịch tính biểu thức số học viết hoàn toàn bằng Kinet**:

```c
// tokenize → parse (recursive descent) → calculate
println(eval_expr("2 + 3 * 4"));     // → 14
println(eval_expr("(2 + 3) * 4"));   // → 20
```

Chạy: `cargo run -- examples/mini_interp.kinet`

→ Nghĩa là mày có thể dùng Kinet để viết trình thông dịch/ngôn ngữ khác, công cụ, thư viện... đúng tinh thần "tạo gì cũng được, miễn có trình".

## Giới hạn & bảo mật
- **Sandbox file I/O MẶC ĐỊNH BẬT**: `read_file`/`write_file`/`import` bị khóa trong **thư mục hiện hành**. Đổi root bằng `KINET_SANDBOX_ROOT=/đường/dẫn`; tắt để chạy code tin cậy bằng `KINET_SANDBOX=none`. Chặn cả `..`, đường dẫn tuyệt đối **và symlink** (canonicalize rồi kiểm tra nằm trong root).
- **Trần bộ nhớ**: chuỗi tối đa 64 triệu ký tự, list/map tối đa 2 triệu phần tử — chống `s = s + s` phình RAM (kiểm tra trước khi cấp phát).
- **Ngân sách thực thi**: 20 triệu bước mỗi `run()` — chống `while true` vô hạn. Thuật toán nặng (Sudoku, ma trận) có thể tăng bằng `KINET_MAX_STEPS=200000000`. Rao `20000000` bước thường do **tree-walking + copy list** — tối ưu cấu trúc dữ liệu hoặc tăng ngân sách.
- **`range()`**: tối đa 2 triệu phần tử.
- **Import** an toàn: chặn import vòng (A→B→A), file lỗi không bị đánh dấu "đã nạp" (import lại sau khi sửa vẫn chạy).
- **Scope nhất quán**: `if`/`while`/`for`/`match` đều tạo scope riêng — `let` trong block không rò ra ngoài; còn gán `=` vào biến ngoài thì vẫn được.
- **`return;`** (không giá trị) trả về `nil`.
- **`\$`** viết được ký tự `$` mà không kích hoạt template (`"\${x}"` → in `"${x}"`).

## Template chuỗi, Lambda, Import

**Template chuỗi `${...}`** — chèn biểu thức vào chuỗi:
```c
let msg = "Xin chào ${name}, có ${len(tasks)} việc!";
```

**Lambda (hàm ẩn danh)** — dùng làm giá trị, truyền cho hàm khác:
```c
let d = func(x) { return x * 2; };
map([1, 2, 3], func(x) { return x * x; });   // → [1, 4, 9]
filter([1, 2, 3, 4], func(x) { return x % 2 == 0; });  // → [2, 4]
```

**Import** — chia code thành nhiều file:
```c
import "examples/strings_lib.kinet";
println(double(21));   // hàm từ file import dùng được ngay
```
Đường dẫn tương đối với thư mục hiện hành; import cùng file 2 lần chỉ chạy 1 lần.

Demo đầy đủ: `cargo run -- examples/import_demo.kinet`

**Công cụ thật làm bằng Kinet**: `cargo run -- examples/file_report.kinet` — quét thư mục,
đọc từng file, thống kê dòng/từ, ghi báo cáo. Dùng `list_dir`, `exists`, `read_lines`,
`try/catch`, lambda + `map`/`filter`, template chuỗi, float.

## Cấu trúc code

| Module | Vai trò |
|---|---|
| `src/token.rs` | Định nghĩa các loại token |
| `src/lexer.rs` | Quét text → token |
| `src/ast.rs` | Cây cú pháp trừu tượng |
| `src/parser.rs` | Token → AST (recursive descent) |
| `src/interpreter.rs` | Chạy AST (tree-walking, môi trường biến theo phạm vi, hàm có closure) |
| `src/main.rs` | CLI: file + REPL |

## Ý tưởng phát triển tiếp
- [x] `match` (pattern dispatch: literal / binding / wildcard)
- [x] List + hàm dựng sẵn (`len`, `get`, `contains`, `append`)
- [x] `for` loop + `break`/`continue`
- [x] Hàm string (`split`, `starts_with`, `ends_with`, `trim`, `replace`, `contains`)
- [x] Cờ debug `--tokens` / `--ast` (xem pipeline)
- [x] Unit test (65 test: lexer, parser, interpreter)
- [ ] Kiểu `float` (hiện chỉ có `i64`)
- [x] **Mốc 4:** Kiểu `map`, file I/O, `chars`/`ord`/`to_int`, đệ quy sâu 2000 — đủ lập trình đa dụng; có `mini_interp.kinet` (viết trình thông dịch bằng Kinet)
- [x] **Giai đoạn A (phần 1):** dict literal `{...}`, comment khối `/* */`, helpers (`substring`, `to_upper`, `to_lower`, `list_join`)
- [x] **Giai đoạn A (phần 2):** stack trace khi lỗi (hiện chuỗi hàm gọi `a → b → c`)
- [x] **Bảo mật & độ chắc:** ngân sách thực thi (chống `while true`), cap `range()`, `return;`, scope nhất quán, sandbox file I/O qua `KINET_SANDBOX_ROOT`, sửa overflow `range`/`substring`, `list_join` đúng chuỗi
- [x] **Template chuỗi `${x}` + Lambda + Import** (Giai đoạn A xong; mở Phase B)
- [x] **Audit v3:** sandbox mặc định bật (chống `..`, path tuyệt đối, symlink), trần bộ nhớ (chuỗi/list/map), `\$` escape, range checked_sub, import chống vòng + không nhớ lỗi
- [x] **Dọn nợ:** stack trace CÓ SỐ DÒNG (`Lỗi dòng N` + `tại hàm x (dòng M)`), map chuyển `IndexMap` (tra cứu O(1))
- [x] **`try/catch`** (bắt lỗi runtime, bind thông báo kèm dòng) + **`float f64`** (số thập phân, trộn được với int)
- [ ] Compile sang bytecode (thay vì tree-walking) để nhanh hơn

### Ghi chú thiết kế (đã cân nhắc, không phải bug cấp bách)
- **`case x =>`** luôn là pattern **binding** (bắt giá trị vào biến x), không so sánh với biến x đã có — tránh bẫy, hãy dùng `case` với literal hoặc `_`/`default`. Nếu cần so sánh với biến đã có, đặt literal/vế so sánh: `case "giá_trị_mong_muốn" => ...`.
- **Lỗi là chuỗi** (chưa phải struct): stack trace ghép chuỗi; trong thực tế hiếm khi người dùng in nội dung chứa đúng `"↳ tại hàm"`. Refactor sang struct là một thay đổi rộng, để khi cần nâng cấp hệ thống lỗi (throw/catch).
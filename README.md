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
```

### Kiểu dữ liệu hiện có
| Kiểu | Ví dụ |
|---|---|
| Số nguyên `i64` | `42`, `-7` |
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
| `to_int(s)` | chuỗi → số nguyên | `to_int("42")` → `42` |
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
- Logic: `and` `or` `not` (đánh giá lười — short-circuit)
- Phép gán: `=`

## Gọi agent Kinetic SDK thật (Mốc 3)

Kinet có native function `agent_run(goal)` để chạy agent Kinetic SDK thật, qua bridge Python ở `bridge/kinet_agent.py`:

```c
// goal-completion loop: hàm viết bằng Kinet, agent chạy SDK thật bên trong
func run_agent(goal, max_rounds) {
    for round in range(0, max_rounds) {
        let result = agent_run(goal);              // gọi Agent(...).run(goal)
        if starts_with(result, "goal complete") {
            return round + 1;
        }
    }
    return 0;
}
```

Chạy demo: `cargo run -- examples/agent_real.kinet`

Ví dụ **script agent đời thực**: `cargo run -- examples/sdk_workflow.kinet` — đọc danh sách
task từ file → chạy agent thật từng task → gom kết quả vào list các map → ghi báo cáo ra file.

### Cấu hình biến môi trường
| Biến | Ý nghĩa | Mặc định |
|---|---|---|
| `KINET_SDK_DIR` | thư mục chứa gói `kinetic_sdk` | `/tmp/kinetic-sdk` |
| `KINET_BRIDGE` | đường dẫn `kinet_agent.py` | `bridge/kinet_agent.py` |
| `OPENAI_API_KEY` | nếu có → dùng `LiteLLMClient` (LLM thật) | không có → `MockLLMClient` offline |
| `KINET_MODEL` | model khi dùng LLM thật | `gpt-4o-mini` |

> `agent_run` hiện gọi bridge Python qua tiến trình (`std::process::Command`, không dùng shell nên tránh được injection). Bước nâng cấp tiếp theo là embed trực tiếp qua PyO3 (như `_kinetic_rs` của SDK) để bỏ chi phí spawn tiến trình mỗi lần gọi.

## Viết phần mềm khác bằng Kinet (Mốc 4)

Kinet đã đủ tính năng cho **lập trình đa dụng**: `map` (dictionary), file I/O, đệ quy sâu (độ sâu 2000), hàm chuỗi, list. Bằng chứng là `examples/mini_interp.kinet` — **một trình thông dịch tính biểu thức số học viết hoàn toàn bằng Kinet**:

```c
// tokenize → parse (recursive descent) → calculate
println(eval_expr("2 + 3 * 4"));     // → 14
println(eval_expr("(2 + 3) * 4"));   // → 20
```

Chạy: `cargo run -- examples/mini_interp.kinet`

→ Nghĩa là mày có thể dùng Kinet để viết trình thông dịch/ngôn ngữ khác, công cụ, thư viện... đúng tinh thần "tạo gì cũng được, miễn có trình".

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
- [x] Unit test (53 test: lexer, parser, interpreter)
- [ ] Kiểu `float` (hiện chỉ có `i64`)
- [x] **Mốc 3:** Kinet điều khiển agent Kinetic SDK thật (agent_run → bridge Python)
- [x] **Mốc 4:** Kiểu `map`, file I/O, `chars`/`ord`/`to_int`, đệ quy sâu 2000 — đủ lập trình đa dụng; có `mini_interp.kinet` (viết trình thông dịch bằng Kinet)
- [x] **Giai đoạn A (phần 1):** dict literal `{...}`, comment khối `/* */`, helpers (`substring`, `to_upper`, `to_lower`, `list_join`), `sdk_workflow.kinet` script agent đời thực
- [ ] Giai đoạn A (phần 2): template chuỗi, bridge có cấu trúc + giữ tiến trình, stack trace
- [ ] Nâng bridge lên embed PyO3 trực tiếp (bỏ spawn tiến trình)
- [ ] Compile sang bytecode (thay vì tree-walking) để nhanh hơn
- [ ] Sandbox + bảo mật (giới hạn quyền truy cập)
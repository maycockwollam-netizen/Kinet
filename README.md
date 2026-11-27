# Kinet — My Programming Language

Một ngôn ngữ lập trình nhỏ viết bằng **Rust**, tự xây dựng theo kiến trúc kinh điển:

```
Code text → Lexer → Token → Parser → AST → Interpreter → Kết quả
```

## Chạy

```bash
cargo run                     # mở REPL
cargo run -- file.kinet         # chạy file source
cargo run -- -e "code"        # chạy code từ dòng lệnh
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
| Hàm | `func f(x) { ... }` |
| Nil | giá trị trả về mặc định |

### Hàm dựng sẵn (native)
| Hàm | Ý nghĩa | Ví dụ |
|---|---|---|
| `len(x)` | độ dài chuỗi hoặc list | `len("chao")` → `4` |
| `get(list, i)` | lấy phần tử (0-based) | `get([10, 20], 1)` → `20` |
| `contains(list, x)` | có chứa phần tử không | `contains([1, 2], 2)` → `true` |
| `append(list, x)` | list mới + thêm phần tử (thuần, không đổi gốc) | `append([1], 2)` → `[1, 2]` |

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
- [ ] Kiểu `float` (hiện chỉ có `i64`)
- [ ] `for` loop, `break`/`continue`
- [ ] Thêm hàm string (split, contains, starts_with...)
- [ ] **Nối Kinetic SDK** qua PyO3 (như `_kinetic_rs`) — Kinet điều khiển agent thật
- [ ] Compile sang bytecode (thay vì tree-walking) để nhanh hơn
- [ ] Sandbox + bảo mật (giới hạn quyền truy cập)
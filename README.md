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
cargo run -- --vm file        # chạy bằng bytecode VM (phase 2)
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
} finally {
    println("Dọn dẹp xong");   // luôn chạy dù lỗi hay thành công
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
| `round(x)` / `floor(x)` / `ceil(x)` | làm tròn số | `round(2.6)` → `3` |
| `abs(x)` | giá trị tuyệt đối | `abs(-7)` → `7` |
| `exists(p)` | đường dẫn có tồn tại? | `exists("a.txt")` → `true` |
| `append_file(p, s)` | ghi THÊM vào file | `append_file("log.txt", "x")` |
| `list_dir(p)` | liệt kê tên file (sắp xếp) | `list_dir(".")` |
| `read_lines(s)` | chuỗi → list dòng | `read_lines("a\nb")` → `["a","b"]` |
| `args()` | tham số dòng lệnh (bỏ tên binary) | `args()[1]` |
| `get_env(name)` | biến môi trường ("" nếu thiếu) | `get_env("PATH")` |
| `sleep(ms)` | tạm dừng mili giây | `sleep(1000)` |
| `input_line()` | đọc 1 dòng từ stdin | `let t = input_line();` |
| `assert(x, msg?)` | kiểm tra điều kiện (lỗi nếu sai) | `assert(x > 0, "phải dương")` |
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
- Số học: `+ - * / %` (có `+` cho chuỗi — nối kiểu; `*` lặp chuỗi: `"ab" * 3` → `"ababab"`)
- So sánh: `== != < > <= >=`
- **`in`**: kiểm tra chứa (cùng bậc so sánh) — `3 in [1, 2, 3]`, `"bc" in "abcd"`, `"k" in map`; có `x not in y` (desugar thành `not (x in y)`)
- Bit: `& | << >>` (chỉ số nguyên)
- Logic: `and` `or` `not` (đánh giá lười — short-circuit)
- Phép gán: `=` `a[i] = v` `m["k"] = v` (chỉ số âm được phép: `a[-1] = 9`)
- **`if` là biểu thức**: `let x = if ok { 1 } else { 2 };` (hỗ trợ `else if` lồng)

### Slice và comprehension (giống Python)
```c
let a = [0, 1, 2, 3, 4, 5];
a[1:4];        // → [1, 2, 3]        (từ 1 tới 3)
a[:3];         // → [0, 1, 2]        (thiếu start = từ đầu)
a[2:];         // → [2, 3, 4, 5]     (thiếu end = tới hết)
a[::2];        // → [0, 2, 4]        (bước nhảy 2)
a[::-1];       // → [5, 4, 3, 2, 1, 0] (đảo ngược)
a[-1];         // → 5                (chỉ số âm, đếm từ cuối)
"xin chào"[1:3]; // → "in"           (cắt chuỗi theo ký tự Unicode)

// List comprehension: [expr for x in list if điều_kiện]
[x * x for x in range(0, 6)];                    // → [0, 1, 4, 9, 16, 25]
[x for x in range(0, 10) if x % 2 == 0];         // → [0, 2, 4, 6, 8]
[x + y for x in [1, 2] for y in [10, 20]];       // nhiều vòng lặp

// Dict comprehension: { key: value for ... }
{ i: i * i for i in range(1, 4) };               // → {1: 1, 2: 4, 3: 9}
```

### Phương thức (method) — cú pháp `giá trị.tên(args)`
Mọi phương thức đều là **đường tắt của hàm dựng sẵn** và **THUẦN** (không sửa biến gốc — khớp thiết kế functional: `a.append(x)` trả list mới, còn `a` không đổi).

```kotlin
// Chuỗi
"  hi  ".trim().upper();      // → "HI"         (chuỗi lại được)
"a,b,c".split(",");           // → ["a","b","c"]
"abc".contains("b");          // → true
"abc".ends_with("bc");        // → true
"abab".replace("a", "X");     // → "XbXb"
"42".to_int();  "3.5".to_float();  "abc".len();

// List (thuần)
[1, 2, 3].append(4);          // → [1,2,3,4]     (gốc [1,2,3] vẫn nguyên)
[1, 2, 3].reverse();          // → [3,2,1]
[1, 2, 3].len();  [1,2,3].contains(2);  [1,2,3].get(1);
[1, 2].join("-");             // → "1-2"
[1, 2, 3].map(func(x) { return x * 2; });    // → [2,4,6]
[1, 2, 3, 4].filter(func(x) { return x % 2 == 0; });  // → [2,4]

// Map (thuần)
m.get("k");  m.set("k", v);  m.keys();  m.contains("k");  m.remove("k");  m.len();

// Số
(-7).abs();   2.6.round();   2.6.floor();   2.4.ceil();   3.to_float();
```

### Struct + Method + Magic methods (OOP kiểu value)
`struct` khai báo **kiểu dữ liệu có khóa cố định** kèm phương thức; tạo instance bằng
`Tên(đối số...)` (gán cho field theo thứ tự, thiếu thì dùng mặc định). Struct là
**kiểu value** — gán/dùng được trong list/map, không có alias.

```kotlin
struct Vec2 {
    x = 0;                       // field + giá trị mặc định
    y = 0;
    func mag() { return x * x + y * y; }        // dùng field trực tiếp (hoặc self.x)
    func scaled(k) { return Vec2(x * k, y * k); }
    func __add__(o) { return Vec2(x + o.x, y + o.y); }   // magic: p + q
    func __mul__(k) { return Vec2(x * k, y * k); }        // magic: p * 2
    func __len__() { return 2; }                          // magic: len(p)
    func __str__() { return "V(" + str(x) + ", " + str(y) + ")"; }  // magic: str(p)
    func __eq__(o) { return x == o.x and y == o.y; }      // magic: p == q
    func __getitem__(i) { if i == 0 { return x; } return y; }      // magic: p[0]
    func __setitem__(i, v) { if i == 0 { self.x = v; } else { self.y = v; } return self; }  // p[i] = v
    func __contains__(n) { return n == x or n == y; }     // magic: n in p
}

let v = Vec2(3, 4);
println(v.mag());        // 25
v.x = 10;                // gán field
println(v + Vec2(1, 1)); // V(11, 5)      (dùng __add__)
println(len(v));         // 2             (dùng __len__)
println(str(v));         // V(10, 4)      (dùng __str__)
```

Magic methods có sẵn: `__add__` `__sub__` `__mul__` `__div__` `__mod__` (toán tử,
thử vế trái trước rồi vế phải), `__eq__` (`==`/`!=`), `__len__` (`len(x)`),
`__str__` (`str(x)`), `__getitem__` / `__setitem__` (`x[i]`), `__contains__` (`in`).
`__setitem__` phải **trả về struct mới** (value semantics, giống `map_set`).

### Tuple, hàm trả nhiều giá trị, đối số mặc định & keyword (giống Python)
```kotlin
func divmod(a, b) { return a / b, a % b; }        // trả nhiều giá trị
let (q, r) = divmod(10, 3);                       // bóc tách qua let (q=3, r=1)
let t = (1, "hai", true);                         // tuple literal
t[1];        len(t);        2 in t;              // index / len / in

func connect(host, port = 80, timeout = 5) { ... }  // tham số mặc định
connect("api.x.com");                              // dùng mặc định
connect("x", timeout = 30);                        // keyword args (đặt theo tên)
connect(host = "y", port = 443);                   // không cần đúng thứ tự

for (k, v) in [("a", 1), ("b", 2)] { ... }         // bóc tách trong for
for key in { "x": 1, "y": 2 } { ... }              // duyệt khóa map trực tiếp
```
- Default được đánh giá **lúc định nghĩa hàm** (như Python).
- Giá trị mặc định bị bỏ qua khi truyền; thiếu đối số không có mặc định → lỗi.
- `Name(tên_field = v)` cũng hoạt động khi tạo instance struct.

### Set + helper kiểu Python + cú pháp gọn
```kotlin
let s = {3, 1, 2};                  // set literal (phần tử duy nhất)
len(s);   2 in s;   for x in s      // membership / duyệt
s.add(4);  s.remove(2);  s.union({5, 6});   // method (thuần, trả set mới)

enumerate(["a","b"])       // [(0,"a"),(1,"b")]
zip([1,2], ["x","y"])      // [(1,"x"),(2,"y")]
sorted([3,1,2])            // [1,2,3]
sorted("cba")              // [a,b,c]
reversed([1,2,3])          // [3,2,1]
sum/min/max/any/all        // số học / so sánh / logic

// Ternary kiểu Python + chuỗi so sánh + elif
let grade = "A" if score >= 90 else "B";
1 < x < 10                          // chain so sánh
if x > 10 { } elif x > 3 { } else { }
```

### `*args` / `**kwargs`, `is` / `is not`, `with`
```kotlin
func total(name, *rest) { ... }        // *rest gom đối số vị trí dư thành list
func opts(**kw) { ... }                // **kw gom keyword dư thành map
total("Tổng", 1, 2, 3);                // rest = [1, 2, 3]
opts(a = 1, b = 2);                    // kw = {a: 1, b: 2}

1 is 1;    "a" is not "b"              // `is`/`is not` — so bằng giá trị

// Context manager: struct có __enter__/__exit__
struct Logger {
    log = [];
    func __enter__() { ...; return self; }
    func __exit__(exc) { ...; return true; }   // exc = nil hoặc chuỗi lỗi
}
with Logger() as lg { ... }            // __enter__ -> thân -> __exit__ (kể cả khi lỗi)
```

## Viết phần mềm khác bằng Kinet (Mốc 4)

Kinet đã đủ tính năng cho **lập trình đa dụng**: `map` (dictionary), file I/O, đệ quy sâu (độ sâu 2000), hàm chuỗi, list. Bằng chứng là `examples/mini_interp.kinet` — **một trình thông dịch tính biểu thức số học viết hoàn toàn bằng Kinet**:

```c
// tokenize → parse (recursive descent) → calculate
println(eval_expr("2 + 3 * 4"));     // → 14
println(eval_expr("(2 + 3) * 4"));   // → 20
```

Chạy: `cargo run -- examples/mini_interp.kinet`

→ Nghĩa là mày có thể dùng Kinet để viết trình thông dịch/ngôn ngữ khác, công cụ, thư viện... đúng tinh thần "tạo gì cũng được, miễn có trình".

## Bytecode VM (phase 2) — `--vm`

Mục tiêu mốc C1 trong roadmap: biên dịch AST → bytecode rồi chạy bằng vòng lặp
VM với **call stack tường minh nằm trên heap** — nên **đệ quy sâu không còn tràn
stack OS** (khác tree-walker giới hạn 2000):

```bash
cargo run -- --vm file.kinet        # VM còn tính được sumto(100000)
cargo run -- file.kinet             # tree-walker (mặc định) giới hạn 2000
```

- Phase 1: literal, biến local/global, toán tử, `and`/`or`, `if`/`elif`/`else`,
  `while`, `for` (list/range), hàm + đệ quy, `return`, `print`/`println`,
  list/tuple + `[]`, `let (a, b) = ...`.
- Phase 2 thêm: **map/set literal** `{...}`, `in`, **slice** `a[1:4]`, gán `a[i] = v`,
  **method trên kiểu** (`s.upper()`, `lst.append()`, `m.get()`...), **`match`**,
  **comprehension** (list + dict), **`try`/`catch`/`finally`** (có `throw`).
- Chưa hỗ trợ (báo lỗi rõ): struct/class, closure/lambda, `with` — bổ sung ở
  phase sau.

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
- [x] **`try/catch/finally`** (bắt lỗi runtime, bind thông báo kèm dòng; `finally` luôn chạy dù lỗi hay thành công, được quyền `return`/`throw` override kết quả) + **`float f64`** (số thập phân, trộn được với int)
- [ ] Compile sang bytecode (thay vì tree-walking) để nhanh hơn

### Ghi chú thiết kế (đã cân nhắc, không phải bug cấp bách)
- **`case x =>`** luôn là pattern **binding** (bắt giá trị vào biến x), không so sánh với biến x đã có — tránh bẫy, hãy dùng `case` với literal hoặc `_`/`default`. Nếu cần so sánh với biến đã có, đặt literal/vế so sánh: `case "giá_trị_mong_muốn" => ...`.
- **Lỗi là chuỗi** (chưa phải struct): stack trace ghép chuỗi; trong thực tế hiếm khi người dùng in nội dung chứa đúng `"↳ tại hàm"`. Refactor sang struct là một thay đổi rộng, để khi cần nâng cấp hệ thống lỗi (throw/catch).
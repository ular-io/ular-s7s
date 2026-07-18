# UI 표준 스타일 가이드

TUI 렌더링에서 색상·텍스트 강조·테두리 강조를 일관되게 유지하기 위한 단일
기준 문서. 모든 스타일 상수/헬퍼는 `src/ui/render.rs`에 정의한다. 새 위젯을
추가하거나 색을 바꿀 때는 새 값을 만들기 전에 이 문서의 기존 토큰을 먼저 쓴다.

## 1. 색상 토큰

`src/ui/render.rs` 상단 상수와 `agent_tag` 헬퍼가 유일한 색 정의처다.

| 토큰 | 값 | 용도 |
| :--- | :--- | :--- |
| `ACCENT` | `Color::Black` | 강조(포커스 테두리/타이틀, 헤더, 주요 값) |
| `KEYCOL` | `Color::Rgb(120, 170, 255)` | 상단 단축키 헤더의 키 표기 |
| `DIM` | `Color::DarkGray` | 구분선, 안내 문구 등 낮은 우선순위 텍스트 |
| `USAGE_HIGH` | `Color::Rgb(80, 150, 255)` | 사용량 50% 이상(파랑), ASCII 로고 '7' 부분(밝은 파랑) |
| `USAGE_LOW` | `Color::Rgb(235, 90, 90)` | 사용량 50% 미만(빨강) |
| agent `Claude` | `Rgb(217, 119, 87)` | 테이블 `A`(agent) 태그 `CLD` |
| agent `Antigravity` | `Rgb(120, 170, 255)` | 테이블 `A` 태그 `AGY` |
| agent `Codex` | `Rgb(140, 220, 160)` | 테이블 `A` 태그 `CDX` |

- 새 색이 필요하면 임의 RGB를 인라인하지 말고 상수로 승격한 뒤 이 표에 추가한다.
- 터미널 테마 차이는 RGB 미세조정이 아니라 아래 "속성"(BOLD/DIM/REVERSED)으로 먼저 해결한다.
- `Color::LightBlue` 같은 16색 기본 ANSI 컬러는 사용자 터미널 테마에 따라 정상적인 밝은 파란색으로 렌더링되지 않거나 무시될 수 있으므로, 로고 등의 밝은 파란색 강조에는 반드시 트루컬러 RGB 값인 `USAGE_HIGH`를 사용한다.

## 2. 텍스트 강조 (Modifier)

| 상태 | 스타일 | 예 |
| :--- | :--- | :--- |
| 강조 값 | `fg(ACCENT) + BOLD` | Prompt 헤더 `Name` 값, `Project` 폴더명, 테이블 헤더 |
| 비강조/부가 정보 | `soft_dim_style()` = `fg(Gray) + DIM` | Prompt 헤더 레이블 전체, `Created/Updated/Id` 값, full path |
| 낮은 우선순위 텍스트 | `fg(DIM)` | 구분선, 상태바 안내, "No sessions" |

- 정보 계층은 **레이블은 항상 `soft_dim_style`**, **핵심 값만 강조**로 표현한다.
  부가 메타(생성/수정 시각, ID, 경로)는 값도 `soft_dim_style`로 눌러 둔다.
- `BOLD`는 "지금 사용자가 봐야 할 한 가지"에만 쓴다. 남발하면 계층이 무너진다.

## 3. 테두리 강조 (BorderType)

포커스/활성 여부는 **색(ACCENT) + 굵기(BorderType)** 이중 신호로 구분한다.
색만으로 구분하면 흑백·색약 환경에서 약하므로 굵기를 함께 바꾼다.

| 상태 | BorderType | border/title 스타일 |
| :--- | :--- | :--- |
| 포커스/활성 | `Thick` (굵은선) | `fg(ACCENT) + BOLD` |
| 비포커스 | `Plain` (얇은선) | `Style::default()` (기본색, **흐림 처리 안 함**) |

- 두 패널(Session/Prompt) 공용 컨테이너: `titled_block(title, focused)`.
- 상시 활성 UI(검색창, 모든 대화상자)는 항상 `Thick` 강조 테두리:
  - `draw_search_prompt`의 Search 블록 (`ACCENT`)
  - `modal_block`(Agent/Folder 필터, Rename) (`ACCENT`)
  - `draw_rename_modal`의 입력 블록 (`ACCENT`)
  - `draw_delete_confirm` (`Color::Red` — 파괴적 동작)
  - `draw_message_modal`(범용 알림) — 심각도별 강조색(아래 참조)
- **심각도색**: 알림/확인 대화상자는 의미에 따라 테두리·버튼색을 바꾼다.
  `MessageKind::Info`→`ACCENT`, `Warn`→`Color::Yellow`, `Error`→`Color::Red`.
  파괴적/차단 상황은 `Red`, 주의는 `Yellow`, 단순 안내는 `ACCENT`.
- 비포커스 패널 테두리는 **흐리게 하지 않는다.** 과거 `soft_dim_style` 적용은
  원복됨 — 흐림은 텍스트 계층 표현에만 쓰고, 테두리 구분은 굵기로 한다.

### 3.1 대화상자 내부 구분선, 패딩 및 버튼 디자인 규칙

대화상자(모달)의 시각적 안정성과 완성도를 높이기 위해 다음 규칙을 준수한다.

* **테두리 밀착 및 두께 일치**:
  * 모달의 하단 구분선은 모달 좌우 테두리에 빈틈없이 밀착하여 닿아야 한다.
  * 모달 블록은 좌우 1글자 마진이 적용된 영역(`block_area`)에 그려지므로, 구분선 X좌표도 `area.x + 1`, 너비는 `area.width - 2`로 맞춘다.
  * 모달 테두리가 `Thick + BOLD`이므로, 구분선도 굵은 선 기호(`┣`, `━`, `┫`)와 `Modifier::BOLD`를 사용하여 두께를 완벽히 통일한다.
* **하단 패딩 및 버튼 여백 구조 통일**:
  * 대화상자 하단의 불필요한 빈 여백을 제거하기 위해 하단 패딩을 `0`으로 지정한다 (`Padding::new(1, 1, 1, 0)`).
  * 하단 패딩이 `0`이므로 하단 버튼 행 바로 아래는 테두리에 밀착하며, 버튼 **바로 위쪽에는 반드시 1줄의 빈 여백**(`Constraint::Length(1)`)이 위치하도록 구성한다.
  * 오류/안내 메시지가 있을 경우 대화상자의 세로 높이(`h`)와 constraints를 동적으로 늘려(예: 12줄 -> 13줄), 오류 메시지와 버튼 행 사이에도 1줄 여백 구조가 유지되도록 방지한다.
* **대화상자 버튼 색상 및 순서 통일**:
  * 모든 대화상자의 버튼 스타일은 동일한 색상 체계를 사용한다.
    * **포커싱(focused)** 상태: 글자색 흰색(`Rgb(255, 255, 255)`), 배경색 밝은 파란색(`Rgb(80, 150, 255)`), Bold 효과를 지정한다.
    * **비포커싱(unfocused)** 상태: 글자색 다크그레이(`Color::DarkGray`), 배경색 밝은 회색(`Color::Gray`)을 지정한다.
  * 버튼의 배치 순서는 **[확인/실행]  [취소]** 순서로 통일한다 (예: `[OK] [Cancel]`, `[Save] [Cancel]`, `[Delete] [Cancel]`).
* **동적 높이 조절 및 붕괴 방지**:
  * 대화상자 내 컨텐츠(폴더 목록 등)의 개수에 맞춰 모달 세로 높이(`h`)를 동적으로 계산해 하단 빈 공간이 남지 않도록 한다.
  * 검색 결과가 `0`개여서 컨텐츠가 없을 때 레이아웃이 뭉개지는 현상을 막기 위해, 최소 높이(기본 오프셋 포함 최소 `10` 이상)를 강제 보장(`clamp(10, max_h)`)하여 빈 목록 공간 최소 1칸을 확보한다.
* **Enter 단축키 오동작 차단**:
  * 텍스트 입력창 등에 포커스가 있을 때 Enter 키를 치는 것으로 폼이 바로 확정되는 단축키 동작을 차단한다.
  * 폼의 확정은 사용자가 직접 하단 버튼 행으로 포커스를 이동시킨 뒤, **확인/실행 버튼이 활성화된 상태에서만 Enter 키로 동작**하도록 이벤트를 처리한다.
  * 취소(Cancel) 버튼 포커스 상태에서의 Enter 키는 대화상자를 닫는 취소 동작으로 작동해야 한다.


## 4. 선택 행 (Table row highlight)

| 상태 | 배경 | 전경/속성 |
| :--- | :--- | :--- |
| Session 포커스 | `Color::Cyan` | `fg(Black) + BOLD` |
| Session 비포커스(Prompt 활성) | `Rgb(55,55,55)` | `soft_dim_style + REVERSED` |
| 그 외 | `DIM` | `fg(Black) + BOLD` |

- 비활성 패널을 표현할 때는 테두리·타이틀·헤더·일반 행·선택 행을 **한 세트로**
  점검한다. 선택 행 하이라이트가 마지막에 덮어쓰므로 `row_highlight_style`을
  먼저 확인한다. (배경: [Panel Focus Style](./panel-focus-style.md))

### 4.1 목록 항목 및 footer 배치 규칙

* **항목 이름 우측 여백**:
  * 폴더/세션 목록 등 텍스트가 길어질 수 있는 항목은 이름 우측에 1글자 여백(` `)을 추가해 가독성을 높인다.
  * 이때 선택 반전 바(배경 강조)는 원래의 전체 영역(Inner width)을 가득 채우는 기본 상태를 유지하도록 한다.
* **footer 정보 배치**:
  * 수치/상태 정보(예: `xx matching folders`, 에러 등)는 하단 **좌측**에 배치한다.
  * 사용자 조작 안내 및 단축키 설명은 하단 **우측**에 배치하고 우측 정렬(`Alignment::Right`)한다.


## 5. 구현 위치 요약

| 대상 | 함수 |
| :--- | :--- |
| 색상 상수 / agent 색 | `render.rs` 상단, `agent_tag` |
| 공용 비강조 스타일 | `soft_dim_style` |
| 패널 컨테이너(포커스 분기) | `titled_block_nav` |
| 대화상자 컨테이너 | `modal_block` |
| 범용 알림 대화상자(재사용) | `draw_message_modal` / `App::show_message` |
| 세션 테이블/선택 행 | `draw_table` |
| Prompt 헤더 메타 정보 | `draw_preview` |

## 6. 체크리스트 (스타일 변경 시)

- [ ] 새 색을 인라인하지 않고 상수로 정의했는가.
- [ ] 강조는 `ACCENT + BOLD`, 비강조는 `soft_dim_style`, 테두리는 `Thick/Plain`
      규칙을 따랐는가.
- [ ] 로고의 '7'과 같은 밝은 파란색 강조에 `Color::LightBlue`가 아닌 트루컬러 RGB (`USAGE_HIGH`)를 사용했는가.
- [ ] 포커스 전환(`Tab`)을 여러 번 하며 두 패널 톤이 함께 바뀌는지 확인했는가.
- [ ] 검색창·대화상자가 모두 `Thick` 강조 테두리를 유지하는가.
- [ ] `cargo clippy` 경고 없이 통과하는가.

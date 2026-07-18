# Panel Focus Style

`session` 패널과 `prompt` 패널의 포커스 스타일 규칙, 그리고 이번 변경에서
반복했던 시행착오를 정리한다. 색·굵기·강조의 전체 기준은
[UI 표준 스타일 가이드](./ui-style-guide.md)를 따른다.

## 목표

- `Tab`으로 포커스를 옮겼을 때 활성 패널과 비활성 패널의 상태가 즉시 구분돼야 한다.
- 포커스 구분은 **색(ACCENT) + 테두리 굵기(BorderType)** 이중 신호로 표현한다.
- 비활성 `session` 패널의 본문/선택 행은 같은 옅은 톤(`soft_dim_style`)으로 통일한다.

## 현재 규칙

### 테두리 (두 패널 공통, `titled_block_nav`)

- 포커스: `BorderType::Thick`(굵은선) + `focus_color + BOLD` (일반적으로 `ACCENT`가 주입되나, 세션 테이블 등 개별 색상 지정 가능)
- 비포커스: `BorderType::Plain`(얇은선) + 기본색. **테두리는 흐리게 하지 않는다.**
  (과거 `soft_dim_style`로 흐리게 하던 처리는 원복됨.)

### `prompt` 포커스 시

- `prompt` 패널 테두리/타이틀은 굵은 강조선
- `session` 패널 테두리는 얇은 기본선, 헤더/일반 행/agent tag는 `soft_dim_style`
- 선택 행은 옅은색 계열을 유지한 채 약한 반전(`REVERSED`)만 적용

### `session` 포커스 시

- `session` 패널은 강조 스타일(굵은 테두리) 유지
- 선택 행은 `Color::Cyan` 배경 + 검은 글자 + `BOLD` (전체 테마 색상과 별도로 청록색 고정)
- `prompt` 패널은 얇은 비활성 테두리/타이틀만 표시

## 구현 위치

- 공용 비활성 스타일: `src/ui/render.rs`의 `soft_dim_style`
- 포커스별 테두리 분기: `titled_block`
- `session` 패널 포커스 분기: `draw_table`
- `prompt` 패널 헤더 메타 정보: `draw_preview`
- 긴 프롬프트 생략 줄의 기준 스타일: [Preview Omission Style](./preview-omission-style.md)

## 시행착오

### 선택 행 배경만 흐리게 바꾼 시도

- 처음에는 비활성 `session` 패널에서 선택 행의 배경색만 `Gray` 계열로 낮췄다.
- 실제 결과는 선택 행만 바뀌고, 헤더/본문/테두리는 여전히 강하게 보여서
  패널 전체가 비활성처럼 보이지 않았다.
- 결론: 포커스 스타일은 `row_highlight_style`만 바꿔서는 해결되지 않는다.

### 일반 행 텍스트만 흐리게 바꾼 시도

- 다음에는 본문 텍스트와 헤더 색만 옅게 바꿨다.
- 하지만 선택 행은 별도의 하이라이트 스타일이 계속 덮어써서 여전히 너무 진했다.
- 결론: `Table::style`과 `row_highlight_style`을 같이 설계해야 한다.

### 회색 RGB 값 미세조정

- `Rgb(70, 70, 70)` 같은 값으로 하이라이트 배경을 조정했다.
- 터미널 테마와 `DIM` 해석에 따라 차이가 작거나, 오히려 선택 행이 더 무거워
  보이는 경우가 있었다.
- 결론: RGB 미세조정보다 공통 톤(`Gray + DIM`)을 먼저 고정하고, 선택 상태는
  약한 반전으로 구분하는 쪽이 일관적이다.

### 레이블과 값의 강조 계층

- `prompt` 헤더는 `Project` / `Name` / `Created at` / `Updated at` / `Id` 순.
- 레이블은 전부 `soft_dim_style`. 핵심 값(`Project` 폴더명, `Name` 값)만
  `ACCENT + BOLD`로 강조하고, 부가 메타(`Created/Updated/Id` 값, full path)는
  값도 `soft_dim_style`로 눌러 정보 계층을 만든다.
- `Path` 단독 줄은 제거되고 `Project` 줄의 `폴더명 (full path)` 형태로 통합됐다.

## 재발 방지 규칙

- 비활성 패널 스타일을 바꿀 때는 `테두리`, `타이틀`, `헤더`, `일반 행`,
  `선택 행`을 한 세트로 점검한다.
- 선택 행이 남아 있다면 `row_highlight_style`이 최종적으로 덮어쓰는지 먼저 확인한다.
- 새로운 비활성 톤을 도입하지 말고, 먼저 `soft_dim_style` 재사용 가능성을 검토한다.
- 터미널 색상 차이를 RGB 값만으로 해결하려고 하지 말고 `DIM` 같은 속성을 우선 사용한다.

## 확인 항목

- `Tab`으로 패널 포커스를 여러 번 전환해 포커스 패널만 굵은 강조선이 되는지 확인한다.
- 선택 행이 비활성 상태에서 과하게 튀지 않는지 확인한다.
- `prompt` 헤더에서 `Project` 폴더명·`Name` 값만 강조되고 나머지 메타는
  옅게 표시되는지 확인한다.

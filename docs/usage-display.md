# Usage Display

헤더의 프로필별 남은 사용량(5h/주간) 표시 기능. 구현은 `src/usage.rs`(조회·파싱),
`src/ui/render.rs::usage_spans`(포맷팅).

## 동작 방식

- 각 CLI를 **보이지 않는 PTY**(200×60)에서 구동하고 사용량 명령(claude·agy는
  `/usage`, codex는 `/status`)을 타이핑한 뒤, `vt100`으로 화면을 재구성해 파싱한다.
- 토큰 추출이나 비공식 API 호출 없이 **공식 클라이언트 화면만** 읽는다.
- 앱 시작 시와 `ctrl+u`에서 **전체 프로필을 병렬 조회**한다. config 폴더 없음
  (`MissingDir`)·조회 수단 없음(`Unavailable` — Antigravity는 env 주입이 불가해
  추가 경로 프로필은 조회 불가)도 조회의 일부로 판정해 상태(phase)로 남긴다.
  **렌더 시점 자동 감지는 없다** — 폴더 존재 여부도 명시적 갱신 때만 재검사하며,
  갱신 사이에 폴더가 생기거나 사라져도 화면은 직전 판정을 유지한다.
- 프로필 추가/편집 저장 시에는 전체가 아닌 **저장된 프로필만 증분 조회**한다
  (`App::start_usage_fetch_for`). PTY 조회는 프로필당 수 초가 걸리는 고비용
  작업이라 기존 프로필의 최신 값을 다시 읽지 않는다. 전체 조회가 진행 중이어도
  증분 조회는 별도 채널로 즉시 시작되며, 같은 프로필이 이미 Loading이면 건너뛴다.
- 기본 경로가 아닌 프로필은 `CLAUDE_CONFIG_DIR`/`CODEX_HOME`을 PTY env로 주입해
  해당 구독의 사용량을 읽는다. **기본 경로에는 주입하지 않는다** — 명시하면
  재로그인 화면이 떠서 조회가 실패한다([상세](profiles.md)).
- 상태는 `UsageEntry { phase, last }`로 관리한다. 실패(Failed) 시에는 `last`
  (직전 성공 스냅샷)를 회색으로 유지 표시해 값이 사라지지 않는다.
- 갱신 중(Loading)에는 직전 판정(로그아웃/미설치/폴더 없음 포함)과 무관하게
  **모든 프로필이 일관되게** 헤더 사용량 자리와 프로필 테이블 STATUS 컬럼에
  `Loading...`을 표시한다(직전 값 유무 무관). 프로필 테이블 USAGE 컬럼은
  직전 값을 회색으로 유지한다. 폴더 없음처럼 즉시 끝나는 판정도 조회 스레드가
  **최소 500ms**(`usage.rs::MIN_LOADING`)를 보장해 깜빡임이 인지된다 — 결과가
  같더라도 "점검이 일어났다"는 피드백을 준다. (과거의 스티키 blocked 표시
  — 재조회 중 차단 문구 유지 — 는 이 일관 표시로 대체되어 제거됐다.)
- 새 프로필의 config 폴더가 로그인/trust 미완료면 `untrusted folder (trust prompt)`로
  실패한다. 해당 config로 한 번 직접 실행해 승인하면 해결된다.
- 결과는 스냅샷이며 TUI가 떠 있는 동안 카운트다운이 실시간 감소하지는 않는다.

## 에이전트별 화면 포맷 (실측 기준)

외부 CLI 업그레이드로 포맷이 바뀔 수 있다. 아래는 검증 당시(claude 2.1.202,
agy 1.0.16, codex 0.142.5/0.143.0) 기준이다.

| 항목 | claude `/usage` | agy `/usage` | codex `/status` |
| :-- | :-- | :-- | :-- |
| % 의미 | `N% used` → 남은량 = 100−N | 게이지 `N%` = 남은량 | `N% left` = 남은량 |
| reset 표기 | 절대 시각 | 상대 시간 | 절대 시각 |
| reset 예시 | `Resets 5am (Asia/Seoul)`, `Resets Jul 10 at 5pm (Asia/Seoul)` | `Refreshes in 16h 51m` | `(resets 04:45)`, `(resets Mon 14:30)`, `(resets Mon Jul 10)`, `(resets 17:33 on 15 Jul)` |
| 5h 라벨 | `Current session` | `Five Hour Limit` | `5h limit:` |
| 주간 라벨 | `Current week (all models)` | `Weekly Limit` | `Weekly limit:` |

주의사항:

- **절대 시각은 카운트다운이 아니다.** codex의 `resets 04:45`는 "4시 45분에
  리셋"이라는 뜻이다. `resets 17:33 on 15 Jul`처럼 날짜가 붙으면 해당 날짜의
  절대 시각으로 보고, 로컬 시각 기준 다음 도래 시점으로 해석해 카운트다운으로 변환한다.
- **agy는 모델 그룹이 2개다** (`GEMINI MODELS` / `CLAUDE AND GPT MODELS`).
  화면 하단 상태줄의 활성 모델명으로 그룹을 골라 읽는다.
- **agy 5h `Disabled`**: 주간 한도 소진 시 5h 한도가 비활성화되고
  `Disabled: … will fully refresh in 16 hours, 37 minutes.` 안내문이 뜬다.
  이때 남은량 0% + 안내문의 refresh 시점을 카운트다운으로 쓴다.
  이 안내문(~150자)이 잘리지 않도록 PTY 폭을 200으로 잡는다.

## 표시 형식

모든 상태에서 고정폭 컬럼을 유지해 행 간 세로 정렬이 맞는다:

```
<1> Claude        72%(4h 30m)  52%(2d 16h) left
<2> Antigravity    0%(17h  6m)   0%(   17h) left
<3> Codex         95%(4h 15m)  51%(    2h) left
```

- %는 폭 3 우측 정렬. 50% 이상 파랑, 미만 빨강. 로딩은 스피너(`✽✻✶%`), 실패는 `--%`.
- 헤더에서는 current(5h) 또는 weekly 중 하나라도 `0%`이면 두 사용량 세그먼트를 모두
  `left`와 같은 dim gray(`Color::Gray` + `Modifier::DIM`)로 표시한다.
- current(5h) 카운트다운: `(4h 30m)` — 분 폭 2 우측 정렬.
- weekly 카운트다운: 분 생략, `(2d 16h)` / `(   17h)`.
- 프로필 화면 테이블은 사용량을 `5H` / `RESET` / `1W` / `RESET` 네 컬럼으로
  분리해 표시하며, reset 컬럼의 괄호는 생략한다.
- 프로필 화면에서 최신 스냅샷 기준 current(5h) 또는 weekly 중 하나라도 `0%`이면
  해당 row 전체를 `left` 문구와 같은 옅은 회색으로 표시한다.
- **config 폴더 없음 판정 프로필**(`UsagePhase::MissingDir` — 삭제·이름 변경
  등)은 사용량 대신 `Config folder not found`(빨강, `MISSING_DIR_LABEL`)를
  표시한다 — 헤더는 사용량 자리, 프로필 테이블은 USAGE 셀(폭 30)에 표시하고
  STATUS 셀은 `Error`를 유지해 나란히 읽히게 한다(ratatui Table 셀은 옆 컬럼으로
  overflow가 불가해 STATUS 대신 인접 USAGE 셀을 사용). 비활성 프로필이면
  soft dim으로 가라앉힌다. 판정은 조회 시점 기준이며 렌더 시점 `is_dir()`
  검사는 하지 않는다.
- 사용량 갱신 중(`Loading`)에는 `Loading...` 문구만(헤더 사용량 자리·프로필
  테이블 STATUS 셀) 페이드 펄스로 깜박인다. row의 나머지 셀은 깜박이지 않는다:
  보통 → 옅음(fg 60% 감쇠) → 더 옅음(`left` 라벨과 같은 soft dim) →
  안 보임(같은 폭 공백 치환) → 더 옅음 → 옅음 순환.
  한 단계 200ms(주기 1.2초) — 리드로 폴링 주기(100ms, `main.rs`)의 2배로 잡아
  aliasing으로 단계가 건너뛰어지지 않게 한다. 안 보임 단계를 HIDDEN(conceal)
  속성이 아닌 공백 치환으로 처리하는 것은 터미널별 지원 편차 때문이다.
  구현은 `src/ui/render.rs`의 `PULSE_SEQ`/`pulse_span`.

## 검증 방법

파싱 코드나 외부 CLI가 바뀌면 단위 테스트만 믿지 말고 실측 검증한다.

```bash
# TUI 없이 세 에이전트 조회 결과만 출력
cargo build && ./target/debug/s7s --usage-probe

# 각 CLI의 최종 화면 텍스트를 파일로 덤프(파서 디버깅용).
# 파일명은 `<cli>-<슬래시명령>.screen.txt` (예: claude-usage.screen.txt —
# 모델 목록 조회 `claude-model.screen.txt`와 구분, docs/models.md 참고)
mkdir -p /tmp/dump && ULAR_USAGE_DUMP=/tmp/dump ./target/debug/s7s --usage-probe
```

- 프로브 결과의 카운트다운이 실제 CLI 화면과 일치하는지 눈으로 대조한다.
- 절대/상대 표기가 애매하면 **몇 분 간격으로 두 번 캡처**한다: 숫자가 그대로면
  절대 시각, 줄어들면 카운트다운이다.
- 단위 테스트(`src/usage.rs` `tests`)의 픽스처는 실측 화면 캡처를 사용하고,
  시각 의존 로직은 고정 `now`를 주입해 검증한다.

# Model Selection (New Session 모델 드롭다운)

New Session 대화상자의 Model 드롭다운에 표시할 "선택 가능 모델 목록"의
조회·캐시·주입 설계. 구현: `src/models.rs`(조회·캐시), `src/ui/mod.rs`
(드롭다운 상태·백그라운드 연동), `src/resume.rs::with_model_flag`(명령 주입).

## 모델 목록 열거 방법 (2026-07 실측)

| 에이전트 | 방법 | 값 형식 | 기본 모델 출처 |
| :-- | :-- | :-- | :-- |
| claude | PTY로 `/model` 화면 스크래핑 (`usage::drive_screen` 공용) | alias 소문자 (`fable`) | 화면 목록의 `✔` 표시 |
| codex | `codex debug models` JSON (`visibility=="list"`만) | slug (`gpt-5.6-sol`) | `<CODEX_HOME>/config.toml` 최상위 `model` 키 |
| agy | `agy models` 줄 단위 출력 | 표시명 그대로 (`Gemini 3.1 Pro (Low)`) | `settings.json` 최상위 `model` 키 |

- claude만 열거 명령이 없어 PTY가 필요하다(프로필당 부팅 수 초). 목록은
  플랜/계정에 따라 다를 수 있어 **프로필별**(`CLAUDE_CONFIG_DIR` 주입) 조회.
- claude `/model` 화면의 `Default (recommended)` 행은 s7s 드롭다운의 자체
  Default(미주입) 항목과 중복이라 목록에서 제외한다. `✔`가 이 행에 있으면
  기본 모델을 None(CLI Default)으로 둔다.
- codex도 프로필별(`CODEX_HOME` 주입) 조회지만 빠른 subprocess다. 카탈로그는
  빈 CODEX_HOME에서도 출력됨을 확인(번들 카탈로그).
- agy는 config env 주입이 불가해(아래 "agy env 주입 검증") **기본 경로 프로필만
  전역 1회** 조회하고, 추가 agy 프로필은 그 결과를 공유한다
  (`ModelCatalog::for_profile` 폴백).

## CLI가 모델명을 검증하지 않는다 (실측)

- agy: 무효 모델명을 주면 **오류 없이 기본 모델로 조용히 폴백**한다.
- codex: 부팅 시 검증 없이 무효 slug를 그대로 표시한다(첫 메시지에서 실패).
- 따라서 **목록 정확성은 s7s 책임**이다: 동적 열거 결과만 드롭다운에 올리고,
  조회 실패 시 기존 캐시를 덮지 않으며(빈 목록 저장 금지), 설정 기본 모델이
  목록에 없으면 OK를 비활성화한다(아래 UI 규칙).

## 캐시와 갱신 시점

- 캐시: `~/.config/s7s/models.json` (profile id 키, `ModelCatalog`).
  항목에 조회 시점 CLI 버전(`--version` 첫 줄)을 함께 저장한다.
- **앱 시작**: 백그라운드 조회를 시작하되 **버전 게이트** — 캐시된 CLI 버전과
  현재 버전이 같으면 재조회를 생략(`ModelsResult::Skipped`)해 claude PTY
  부팅 비용을 없앤다. 모델 목록은 CLI 업그레이드/플랜 변경 때만 바뀐다.
- **ctrl+u**: 강제 재조회(버전 게이트 무시) — 플랜 변경까지 커버.
- **프로필 저장**: 저장된 프로필만 증분 강제 조회(path가 바뀌었을 수 있음).
- **프로필 삭제**: 캐시 항목 제거.
- 사용량 조회와 달리 **조용히** 진행한다: Loading 표시·완료 메시지 없음.
  사용량 `Loading...`이 사라져도 모델 조회는 계속될 수 있다.
- 로그인 안 됨/폴더 없음/CLI 미설치는 `Unavailable`로 스킵(캐시 유지).

## New Session 대화상자 UI 규칙

- 컨트롤 순서(tab/↑↓): Profile → **Model** → Folder → OK/Cancel. 모달 높이 14행.
- 드롭다운 0번은 항상 **Default**(`--model` 미주입 — CLI 자체 기본 모델 사용).
- 초기 선택 = CLI 설정의 기본 모델. 목록에 없으면 **missing 자리표시 항목**
  (빨강, "not in the fetched model list")을 선택해 두고 **OK 비활성화** —
  사용자가 다른 항목(Default 포함)을 골라야 실행 가능(오타/낡은 설정을
  조용히 실행하지 않기 위함).
- 프로필 확정 변경 시 모델 항목을 해당 agent 기준으로 재구성한다.
- 백그라운드 조회 완료가 **열려 있는 대화상자의 목록을 즉시 교체하지 않는다**
  (커서 점프 방지) — 다음에 열 때 반영된다.
- 캐시가 전혀 없으면 내장 폴백: claude만 alias 4종(fable/opus/sonnet/haiku).
  codex/agy는 열거가 빨라 폴백 없이 Default만 표시된다(첫 조회 후 채워짐).
- resume(이어하기)의 모델 선택은 미구현(2026-07-14 결정: 추후 별도).

## 명령 주입 (append 방식)

- `NewSessionRequest.model`(Option) → `resume::run_new`/`preview_new_command`가
  템플릿 꼬리에 ` --model '<값>'`을 덧붙인다. Default(None)면 그대로.
- 템플릿(`config.toml`의 `new_*`)은 손대지 않으므로 기존 사용자 설정과 호환.
- 값은 항상 작은따옴표 쿼팅(agy 표시명의 공백·괄호 대비).
- 세 CLI 모두 `--model` 장플래그 동작을 부팅 배너/상태줄로 실측 확인:
  claude alias·전체명(`claude-haiku-4-5-20251001`), codex slug, agy 표시명.

## Add Profile에서 Antigravity 차단

- agy는 추가 프로필이 무의미(env 주입 불가 → usage 스킵·resume 기본 계정)라
  **신규 추가/타 에이전트에서의 전환 시 Antigravity 라디오를 dim + 선택 불가**
  처리한다(`ProfileFormState::agy_allowed`, 저장 단계 방어 검증 포함).
  기존 Antigravity 프로필의 편집·삭제는 유지된다. builtin agy 프로필은
  시드로 항상 존재하므로 접근성 손실이 없다.

### agy env 주입 검증 (2026-07-14, agy 1.1.2)

- `strings $(which agy)`의 `ANTIGRAVITY_*` 환경변수 전수 목록에
  `ANTIGRAVITY_CONFIG_DIR` **없음**(서드파티 문서의 해당 변수는 이 CLI에
  코드 경로가 존재하지 않음).
- 빈 폴더를 `ANTIGRAVITY_CONFIG_DIR`로 지정해 부팅해도 기존 계정으로 뜨고
  폴더는 빈 채로 남음 — **완전 무시 확인**.
- `HOME` 오버라이드는 동작하지만(새 `.gemini` 트리 생성 + 로그인 플로우)
  에이전트 작업 환경 왜곡·키체인 계정 충돌 미검증으로 채택하지 않음.
- agy 업그레이드 시 `strings $(which agy) | grep -o 'ANTIGRAVITY_[A-Z_]*'`로
  전용 변수 신설 여부를 재확인하고, 생기면 차단을 해제한다.

## 검증 방법

모델 파싱 코드를 바꿨거나 agent CLI가 업그레이드됐다면:

```bash
# TUI 없이 전체 프로필의 모델 목록 강제 조회(캐시 미갱신)
cargo build --release && ./target/release/s7s --model-probe

# claude /model 화면 텍스트 덤프(파서 디버깅용, claude-model.screen.txt)
mkdir -p /tmp/dump && ULAR_USAGE_DUMP=/tmp/dump ./target/release/s7s --model-probe
```

- claude는 실제 `claude`에서 `/model`을 열어 목록·✔ 위치와 대조한다.
- codex는 `codex debug models` 출력(visibility=list)과, agy는 `agy models`
  출력과 대조한다.
- CLI가 무효 모델명을 걸러주지 않으므로, 프로브 결과의 value로 실제 세션을
  한 번 띄워 배너/상태줄의 활성 모델 표기를 확인하는 것이 최종 검증이다.
- 단위 테스트(`src/models.rs::tests`)의 claude 화면 픽스처는 실측 캡처
  (2026-07-14, 2.1.207)를 사용한다. 화면 포맷이 바뀌면 픽스처도 갱신한다.

## New Session with Context와 모델 선택

컨텍스트 첨부 새 세션([상세](session-context.md))은 기존 New Session 대화상자를
그대로 재사용하므로 Model 드롭다운의 동작(목록 출처·기본 선택·missing 처리)도
변경 없이 동일하다. 선택된 모델의 `--model` 플래그는 부트스트랩 프롬프트보다
앞에 주입된다(`<템플릿> --model '<값>' '<프롬프트>'`).

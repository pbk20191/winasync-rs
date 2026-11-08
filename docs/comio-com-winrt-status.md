# comio COM/WinRT 노출 현황

*기준일: 2026-11-09*

## 현재 상태

comio의 객체는 **WinRT ABI로 구현된 클래식 COM 컴포넌트**다. 아래 네 겹이 모두 동작하며, 각각 다른 청중을 위한 것이다.

| 겹 | 무엇을 하나 | 모듈 |
| --- | --- | --- |
| 활성화 | CLSID/ProgID로 `CoCreateInstance`. `DllRegisterServer`가 HKCU에 per-user 등록 (승격 불필요) | `activation.rs` |
| IDispatch | 스크립트용 늦은 바인딩. 블로킹 모양 — `Connect` / `Accept(ms)` / `WriteText` / `ReadText` | `dispatch.rs` |
| 타입 라이브러리 | dispatch 테이블에서 `.tlb`를 **생성**. 이른 바인딩 + coclass의 `[default, source]` | `typelib.rs` |
| 커넥션 포인트 | `IConnectionPointContainer` + `IProvideClassInfo2`. 싱크에 `ConnectionReceived` 전달 | `connpt.rs` |

`.tlb`를 IDL이 아니라 기존 테이블에서 생성하므로, 이른 바인딩과 늦은 바인딩이 멤버를 다르게 기술하는 일이 **구조적으로 불가능**하고 빌드에 `midl.exe`가 필요 없다.

빠져 있는 것은 WinRT 쪽 윗겹이다: `.winmd` 메타데이터가 없고, 활성화도 `RoActivateInstance`가 아니라 `CoCreateInstance`다. 그래서 vtable의 `IAsyncOperation` 비동기 모양은 Rust 외의 언어에서 읽을 방법이 없다.

## 호스트별 도달 범위

스크립트 호스트는 이벤트까지 전부 도달하고, 비동기 vtable은 Rust만 읽는다.

| 호스트 | 객체 생성 | 메서드 호출 | 이벤트 | `IAsyncOperation` |
| --- | --- | --- | --- | --- |
| WSH / VBScript | 검증 | 검증 | **검증** (`WScript.ConnectObject`) | 불가 |
| PowerShell 7 | 검증 | 검증 | **검증** — tlbimp interop 필요 | 불가 |
| C# / .NET | 됨 (tlbimp) | 됨 | 됨 — 같은 interop 경로 | 불가 (`.winmd` 없음) |
| C++ | 됨 (`#import` / vtable) | 됨 | 됨 | 수동 선언하면 |
| VBA | 됨 | 됨 | 같은 탐색 경로 (미검증) | 불가 |
| Rust | 직접 | 직접 | 직접 | **직접** |

"검증"은 실제로 돌리고 출력을 확인한 것이다. VBA는 WSH와 **동일한 탐색 경로**(class info → default source → `FindConnectionPoint`)를 쓰지만 직접 돌려보지는 않았다.

### PowerShell 레시피

`Register-ObjectEvent`는 `Type.GetEvent`로 이름을 찾는데 `System.__ComObject`는 답할 게 없다. interop 어셈블리가 그걸 메운다.

```text
regsvr32 /s rustwinui3loop.dll
TlbImp.exe "%LOCALAPPDATA%\rustwinui3loop\rustwinui3loop.tlb" ^
    /out:Rustwinui3loop.Interop.dll /namespace:Rustwinui3loop
```

```powershell
Add-Type -Path .\Rustwinui3loop.Interop.dll
# -ComObject 가 아니라 interop coclass 여야 한다
$l = New-Object Rustwinui3loop.NamedPipeConnectionListenerClass
Register-ObjectEvent -InputObject $l -EventName ConnectionReceived -SourceIdentifier conn -Action {
    $EventArgs.ReadText(64)
}
$l.Bind($name)
```

주의 두 가지.

- `ComEventsHelper.Combine`으로 내려가면 advise는 성공하지만 **싱크가 runspace 없는 스레드에서 scriptblock을 불러 죽는다**. `Register-ObjectEvent`만 동작한다.
- 전달은 **협조적**이다. PowerShell이 이벤트를 자기 큐에 넣고 파이프라인 스레드의 문장 경계에서 실행하므로, 메인이 긴 호출 하나에 막히면 제 핸들러도 지연된다. 그럴 땐 `Start-ThreadJob` 안의 블로킹 `Accept`가 맞다.

tlbimp은 .NET Framework SDK 도구다. **빌드 단계에 넣지 않는다** — midl을 피한 이유를 그대로 되살리게 된다. 소비자 쪽 선택적 단계로 둔다.

## 측정으로 밝혀진 것

세 건 모두 **in-process 테스트는 전부 통과하는데 외부 호스트에서만 깨지는** 종류였다.

### `CO_E_NOTINITIALIZED` — 싱크 호출이 조용히 실패

스레드풀 스레드는 `CoInitializeEx`가 불린 적이 없다. 이런 스레드는 **암묵적 MTA**가 서비스하는데, 그건 MTA가 존재할 때만이다. `cscript`처럼 호스트 스레드가 전부 STA면 프로세스에 MTA가 없어 첫 `Invoke`가 실패하고 이벤트가 사라진다.

해법은 첫 `Advise` 때의 `CoIncrementMTAUsage` (`connpt.rs`). 이건 **스레드를 초기화하지 않고** 프로세스에 MTA가 살아있게만 한다. 스레드풀 스레드에 `CoInitializeEx`를 부르면 안 되는 이유:

- 콜백 끝에 `CoUninitialize`했다가 하필 마지막 MTA 참조였다면 **프로세스 전체의 마샬링된 프록시가 죽는다**.
- 해제하지 않으면 남의 콜백을 실행할 공유 스레드의 아파트먼트 상태를 영구히 바꿔놓는 것이다.

> MS 문서의 "WinRT를 쓰면 `CoInitializeEx` 대신 `Windows::Foundation::Initialize`"는 **내가 소유한 스레드를 초기화할 때**의 이야기다. 우리 콜백 스레드는 WinRT 클래스를 **활성화하지 않고** 이미 존재하는 인터페이스 포인터로 메서드만 부르므로, 아파트먼트만 필요하고 WinRT 초기화는 불필요하다. (`listener.rs:42` 참조 — 객체는 Rust 팩토리에서 오지 `RoActivateInstance`가 아니다.)

### `TYPE_E_LIBNOTREGISTERED` — 타입 라이브러리 LCID

`CreateTypeLib2`가 라이브러리 LCID를 **사용자 기본값**(여기선 412, ko-KR)으로 잡아 그 아래 등록했고, `LoadRegTypeLib`는 중립 0으로 찾아 못 찾았다. `SetLcid(0)`으로 고정. 언어 설정이 다른 장비에서만 재현되었을 버그다. `unregister`는 이제 `TypeLib\{libid}` 서브트리 전체를 지워 낡은 LCID 등록이 살아남지 못하게 한다.

### PowerShell 이벤트 — 틀리게 문서화했던 것

"PowerShell은 이 이벤트를 받을 수 없다"고 적었는데, 그건 **타입 라이브러리를 만들기 전 기준**이었다. `.tlb`를 추가한 순간 tlbimp이 필요한 글루(`DListenerEvents_EventProvider`, 싱크 헬퍼, 각 coclass의 .NET `ConnectionReceived` 이벤트)를 전부 생성할 수 있게 됐는데, 그걸 다시 확인하지 않았다. 문서는 수정했다.

## 막힌 것: `IInspectable`이 비어 있다

우리 객체는 진짜로 `IInspectable`이다 — `IClosable`, `IInputStream`, `IOutputStream`이 전부 WinRT 인터페이스라 QI가 성공한다. 그런데 세 메서드가 전부 `S_OK`로 **빈 값**을 돌려준다. "모른다"가 아니라 "없다"고 자신 있게 답하는 것이라 더 나쁘다.

| 메서드 | 실제 응답 | 어디서 결정되나 | 훅 |
| --- | --- | --- | --- |
| `GetIids` | `count=0`, `values=null` | `IInspectable_Vtbl::new`가 설치하는 **자유 함수**에 하드코딩 | **없음** — 타입 파라미터를 아예 안 받음 |
| `GetRuntimeClassName` | `""` | `#[implement]` 목록의 **첫 인터페이스**의 `RuntimeName::NAME` | 없음 — `#[interface]`가 빈 impl을 자동 생성해 덮어쓰면 충돌 |
| `GetTrustLevel` | `0` (BaseTrust) | 매크로가 직접 넣음 | 없음 — 속성 파서가 인터페이스 경로만 읽음 |

근본 원인은 identity vtable이 생성된 `_Impl` 타입의 **연관 상수**고, `QueryInterface`가 `IID_IInspectable`을 가장 먼저 거기로 보낸다는 것이다. 끼어들 틈이 없다. `windows-implement` 0.100도 구조가 같아 업그레이드로 풀릴 문제가 아니다.

### 반쪽 수정을 안 한 이유

`GetRuntimeClassName`만은 편법이 있다 — `RuntimeName`을 손으로 단 마커 인터페이스를 `#[implement]` 첫 줄에 넣으면 된다. 안 했다.

**런타임 클래스 이름은 "이 이름이 WinRT 메타데이터로 해석된다"는 약속이다.** `.winmd`를 안 내놓으니 `RoGetActivationFactory`는 여전히 실패한다. 이름을 채우면 객체의 주장이 **더 그럴듯해질 뿐 참이 되지는 않는다**. 빈 문자열이 차라리 "해석될 것처럼 생긴 거짓말"은 아니다. 그리고 `GetIids`는 어차피 못 고친다.

즉 이 둘을 정직하게 만드는 일은 **`.winmd` 작업의 일부**이지 그보다 먼저 할 수 있는 단계가 아니다.

### 대신 한 것

`comio::tests::iinspectable_is_shaped_but_hollow` — characterisation 테스트로 현재 응답을 못 박았다. **컨트롤**로 진짜 WinRT 클래스(`Windows.Storage.Streams.Buffer`)를 같은 코드 경로로 통과시켜 `GetIids`가 0이 아니고 이름도 비어 있지 않음을 확인한다. 이게 없으면 "빈 응답"이 호출 방식 탓일 수도 있다. `windows-rs`를 올렸을 때 이게 바뀌면 조용히 지나가지 않고 테스트가 깨진다.

## `.tlb` vs `.winmd`

둘은 대체재가 아니라 **청중이 겹치지 않는다**.

| 소비자 | `.tlb` | `.winmd` |
| --- | --- | --- |
| VBScript/WSH, VBA, Office | **유일한 길** (검증됨) | 불가 |
| PowerShell 7 | tlbimp, 검증됨 | 실질적으로 불가 |
| C# / .NET | 블로킹 모양 | 진짜 `await` |
| C++ | `#import`, 블로킹 | `co_await` |
| Rust | (이미 직접) | 됨 |

### `.winmd`가 더 나쁜 두 지점

1. **배포가 후퇴한다.** WinRT 클래스 활성화는 `HKLM\SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId`(머신 전역, 승격 필요) 아니면 **소비하는 앱 쪽** `application.manifest`의 `<activatableClass>`(reg-free WinRT)다. 지금은 HKCU per-user, 승격 없이 `regsvr32` 한 줄이다.
2. **PowerShell에 도움이 안 된다.** PS 7은 .NET 5+에서 내장 WinRT 지원이 제거됐다. cswinrt로 프로젝션 어셈블리를 생성해야 하는데 그러면 결국 interop 어셈블리고, tlbimp이 더 짧은 경로로 이미 준 것과 같은 자리다. reg-free 활성화를 하려면 `pwsh.exe`에 매니페스트가 필요한데 현실적으로 못 한다. PowerShell엔 `await`도 없어서 `IAsyncOperation`을 얻어도 `.ContinueWith{}`는 runspace 없는 스레드에서 죽는다.

### `.winmd`가 진짜 더 나은 지점

비동기 모델을 **번역하지 않고 그대로 기술**한다. `IAsyncOperation<T>`가 네이티브로 나오고, `Completed`가 콜백이고, `Cancel`과 `ErrorCode`가 딸려오고, 커넥션 포인트도 블로킹 투영도 필요 없다. 컴파일된 소비자에게 `.tlb`가 **번역**인 데 반해 `.winmd`는 **기술**이다. 덤으로 `GetIids`/`GetRuntimeClassName`이 정직해진다.

### 비용

순수 Rust로 가능하다 — `windows-metadata`는 ECMA-335를 읽고 **쓰며**, `windows-rdl`이 RDL → `.winmd` 컴파일을 한다. midl/cl.exe 불필요.

진짜 작업량은 라이터가 아니다. 우리 dispatch 테이블은 **블로킹 dispinterface 모양**을 기술하는데, `.winmd`가 기술해야 하는 건 **비동기 vtable 모양**(`IConnectionListener`, `IAsyncOperation` 반환)이고 그건 Rust `#[interface]` 선언에만 존재한다. 즉 테이블에서 자동 생성이 안 되고 그 표면에 대한 새 기술이 필요하며, `.tlb`에서 구조적으로 막아둔 드리프트 위험이 거기서 다시 들어온다.

**선결 질문 둘**: 비동기 표면의 기술을 어디에 둘 것인가(RDL 파일 vs `windows-metadata` 라이터로 테이블에서 직접 방출), 배포를 HKLM vs 소비자 매니페스트 중 어디로 할 것인가.

## 다음 작업

권장 순서: **1 → 2 → 3 → 4**. 1번으로 사실을 확보하고, 2번으로 지금 설계의 한계를 전부 드러낸 뒤, 3번(확정된 gap)을 하고, 그 다음 4번(제일 큼)으로 간다.

1. **`IAsyncOperation` 교차 아파트먼트 마샬링 측정** · ~30줄
   지금 문서에 추측으로 남아 있는 유일한 항목. STA 스레드를 띄우고 `CoMarshalInterThreadInterfaceInStream`으로 `IAsyncOperation<IInspectable>`을 넘겨 `Completed`를 걸어본다. 어느 쪽 결과든 `.winmd` 판단의 근거가 추측에서 사실로 바뀐다.

2. **out-of-process 활성화 (`LocalServer32`)** · 중간
   지금은 in-proc 전용. **마샬링·아파트먼트·수명을 전부 강제로 통과**시키므로 1번의 상위 집합이고, `.winmd`로 가기 전에 현재 표면의 한계를 남김없이 드러낸다.

3. **hvsock TLS** · 중간
   메모리에 "진짜 gap"으로 남은 유일한 항목(2026-11-08 보류). WinRT의 TLS는 `StreamSocket`의 `SocketProtectionLevel`에만 있고, hvsock은 **host↔guest 신뢰 경계**를 넘으므로 여기서만 실제 필요하다(로컬 파이프엔 무의미). Schannel을 `OverlappedDuplexStream` 위에 얹으면 `IInputStream`/`IOutputStream` 모양이 유지된다.

4. **`.winmd` + `DllGetActivationFactory`** · 큼
   위 "비용" 절의 선결 질문 둘을 먼저 정한다.

5. **오퍼레이션 객체(상관 토큰)** · 작음~중간
   지금 커넥션 포인트 이벤트는 스트림만 주고 식별자를 안 준다. 동시에 여러 작업을 띄운 스크립트가 어느 게 끝났는지 구분 못 한다. 4번을 하면 컴파일된 소비자에겐 불필요해지지만 스크립트에겐 여전히 유일한 수단이다.

6. **tlbimp interop 통합 테스트**(tlbimp 없으면 skip) · 작음
   부수 효과가 본체보다 크다 — **우리 `.tlb`가 MS 표준 툴체인에서 읽힌다는 독립 검증**(지금은 우리 리더로만 확인)과 PowerShell 이벤트 경로 회귀 방지.

### 가치 낮음

- **머신 전역 등록(HKLM)** — 승격 감지 시 분기. 작음.
- **`DllCanUnloadNow` 객체 카운팅** — 지금 무조건 `S_FALSE`. 이유는 `activation.rs` 문서에 있고, 고치면 `CoFreeUnusedLibraries`가 동작. 위험 대비 이득 작음.
- **`EnumConnectionPoints` / `EnumConnections`** — 디자인타임 브라우저 전용. **권장하지 않음**, 소비자가 없다.

### 위생

기존 clippy 경고 24건 (`tp` / `rio` / `main.rs` / `listener.rs`). 이번 작업과 무관한 기존 것들.

## 커밋 상태

| 커밋 | 내용 |
| --- | --- |
| `ace1f38` | 타입 라이브러리와 커넥션 포인트 |
| `c483249` | IDispatch |
| `abdb902` | 활성화 |
| `aa05a16` | 클라이언트 타입을 COM 커넥터로 |

**미커밋** (워킹 트리): `src/comio/activation.rs`, `src/comio/connpt.rs`, `src/comio/mod.rs` — 문서 교정 + `iinspectable_is_shaped_but_hollow` 테스트.

### 재현

```powershell
# 등록 / 해제 (HKCU, 승격 불필요)
Start-Process regsvr32 -ArgumentList '/s', 'target\debug\rustwinui3loop.dll' -Wait
Start-Process regsvr32 -ArgumentList '/s', '/u', 'target\debug\rustwinui3loop.dll' -Wait

# 전체 검증
cargo test --all-features
cargo clippy --all-targets --all-features
```

타입 라이브러리는 `DllRegisterServer`가 `%LOCALAPPDATA%\rustwinui3loop\rustwinui3loop.tlb`에 쓴다.

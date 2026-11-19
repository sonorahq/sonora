equalizer-advanced = 고급
equalizer-simple = 기본
equalizer-sound = 사운드
equalizer-profile-default = 기본값
equalizer-profile-new = 프로필 { $number }
equalizer-profile-pasted = 붙여넣은 프로필
equalizer-profile-name-hint = 프로필 이름
equalizer-save-as = 프로필로 저장
equalizer-rename = 프로필 이름 바꾸기
equalizer-delete = 프로필 삭제
equalizer-bass = 저음
equalizer-treble = 고음
equalizer-tone-reset = 0 dB로 초기화
equalizer-preamp = 프리앰프
equalizer-preamp-auto = 자동
equalizer-preamp-manual = 수동
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = oct
equalizer-no-gain = —
equalizer-filters = 필터
equalizer-no-filters = 필터가 아직 없습니다. 필터를 추가하거나 위에서 프리셋을 선택하거나 프로필을 가져오세요.
equalizer-add-filter = 필터 추가
equalizer-remove-filter = 필터 제거
equalizer-filter = 필터 { $number }
equalizer-filter-kind = 필터 { $number } 유형
equalizer-kind-peak = 피크(Peak)
equalizer-kind-low-shelf = 로우 셸프(Low shelf)
equalizer-kind-high-shelf = 하이 셸프(High shelf)
equalizer-kind-low-pass = 로우 패스(Low pass)
equalizer-kind-high-pass = 하이 패스(High pass)
equalizer-kind-notch = 노치(Notch)
equalizer-kind-band-pass = 밴드 패스(Band pass)
equalizer-kind-all-pass = 올 패스(All pass)
equalizer-transfer = 가져오기 및 내보내기
equalizer-import = 가져오기
equalizer-export = 내보내기
equalizer-import-file = 파일 가져오기…
equalizer-export-file = 파일로 내보내기…
equalizer-imported = { $name } 가져옴
equalizer-imported-skipped = { $count ->
        [one] 1줄을 제외하고 { $name } 가져옴
       *[other] { $count }줄을 제외하고 { $name } 가져옴
    }
equalizer-import-nothing = 가져올 이퀄라이저 설정을 찾지 못했습니다.
equalizer-import-too-many = 가져오기 실패: 필터 { $count ->
        [one] 1개가 켜져 있어
       *[other] { $count }개가 켜져 있어
    } 프로필에 담을 수 있는 한도를 초과했습니다.
equalizer-import-too-large = 가져오기 실패: 텍스트 크기가 1 MB를 초과하여 이퀄라이저 프로필로 너무 큽니다.
equalizer-filter-limit = 프로필에는 최대 { $max }개의 필터를 담을 수 있습니다.
equalizer-paste-empty = 클립보드에 텍스트가 없습니다.
equalizer-copied = Equalizer APO 텍스트 형식으로 프로필을 복사했습니다.
equalizer-exported = 프로필을 내보냈습니다.
equalizer-file-unreadable = 파일을 읽을 수 없습니다.
equalizer-file-unwritable = 파일에 쓸 수 없습니다.
equalizer-value-refused = 이전 값을 유지했습니다.
equalizer-skipped-line = { $line }번째 줄: { $reason }
equalizer-skipped-more = 외 { $count ->
        [one] 1개 더 있음
       *[other] { $count }개 더 있음
    }
equalizer-skip-unknown-type = 알 수 없는 필터 유형 { $name }
equalizer-skip-unreadable = Sonora에서 읽을 수 없는 필터입니다.
equalizer-skip-no-frequency = 주파수 없음
equalizer-skip-no-gain = 게인 없음
equalizer-skip-no-width = Q 값 또는 대역폭 없음
equalizer-skip-expression = 수식으로 작성된 값입니다.
equalizer-skip-no-preamp = 프리앰프 값이 숫자가 아닙니다.
equalizer-skip-unsupported = { $name }은(는) 지원되지 않습니다.
equalizer-rules = 기기 규칙
equalizer-rule-other = 기타 모든 기기
equalizer-rule-playing = 지금 재생 중
equalizer-rule-replaces = 기존 규칙 대체
equalizer-rule-choose-device = 기기 선택
equalizer-mode = 모드
equalizer-preamp-level = 프리앰프 레벨
equalizer-no-gain-spoken = 게인 없음
equalizer-filter-frequency = 필터 { $number } 주파수
equalizer-filter-gain = 필터 { $number } 게인
equalizer-filter-width = 필터 { $number } 대역폭
equalizer-graph = 주파수 응답
equalizer-graph-detail = { $count ->
    [0] 필터 없음
    [one] 필터 1개
   *[other] 필터 { $count }개
    }, 프리앰프 { $db } dB
equalizer-rule-device = 기기
equalizer-rule-choose-sound = 사운드 프로필 선택
equalizer-rule-add = 규칙 추가
equalizer-rule-remove = 규칙 제거
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = 주파수는 { $low }~{ $high } Hz 사이여야 합니다.
equalizer-invalid-gain = 게인은 { $low }~{ $high } dB 사이여야 합니다.
equalizer-invalid-q = Q 값은 { $low }~{ $high } 사이여야 합니다.
equalizer-invalid-octaves = 대역폭은 { $low }~{ $high } 옥타브 사이여야 합니다.
equalizer-invalid-shelf-octaves = 셸프 필터는 대역폭이 아닌 Q 값을 사용합니다.
equalizer-invalid-preamp = 프리앰프는 { $low }~{ $high } dB 사이여야 합니다.
equalizer-overload = { $db } dB 클리핑 발생
equalizer-overload-slight = 0.1 dB 미만 클리핑 발생
equalizer-lower-preamp = 프리앰프 낮추기
equalizer-level = 레벨
equalizer-status-bit-perfect = 비트 퍼펙트
equalizer-status-bit-perfect-detail = Sonora에서 비트 퍼펙트로 출력됨
equalizer-status-server = 이후 사운드 서버에서 { $khz } kHz로 변환됩니다.
equalizer-status-altered = 비트 퍼펙트가 아닌 이유:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = 볼륨
equalizer-status-normalized = 음량 평준화
equalizer-status-direct = Sonora를 거치지 않고 Soloist에서 직접 재생됨
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bit
equalizer-status-float = Float → { $to } bit
equalizer-status-channels = { $from ->
    [1] 모노
    [2] 스테레오
   *[other] { $from }채널
} → { $to ->
    [1] 모노
    [2] 스테레오
   *[other] { $to }채널
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB

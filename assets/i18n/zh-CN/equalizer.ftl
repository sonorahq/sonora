equalizer-advanced = 高级
equalizer-simple = 简单
equalizer-sound = 音效
equalizer-profile-default = 默认
equalizer-profile-new = 配置文件 { $number }
equalizer-profile-pasted = 粘贴的配置文件
equalizer-profile-name-hint = 配置文件名称
equalizer-save-as = 另存为配置文件
equalizer-rename = 重命名配置文件
equalizer-delete = 删除配置文件
equalizer-bass = 低音
equalizer-treble = 高音
equalizer-tone-reset = 重置为 0 dB
equalizer-preamp = 前级放大
equalizer-preamp-auto = 自动
equalizer-preamp-manual = 手动
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = 倍频程
equalizer-no-gain = —
equalizer-filters = 滤波器
equalizer-no-filters = 还没有滤波器。添加一个、选择上方的预设，或导入配置文件。
equalizer-add-filter = 添加滤波器
equalizer-remove-filter = 移除滤波器
equalizer-kind-peak = 峰值
equalizer-kind-low-shelf = 低架
equalizer-kind-high-shelf = 高架
equalizer-kind-low-pass = 低通
equalizer-kind-high-pass = 高通
equalizer-kind-notch = 陷波
equalizer-kind-band-pass = 带通
equalizer-kind-all-pass = 全通
equalizer-transfer = 导入和导出
equalizer-import = 导入
equalizer-export = 导出
equalizer-import-file = 导入文件…
equalizer-export-file = 导出文件…
equalizer-imported = 已导入 { $name }
equalizer-imported-skipped = 已导入 { $name }，跳过了 { $count ->
        [one] { $count } 行
       *[other] { $count } 行
    }
equalizer-import-nothing = 未找到可导入的均衡器设置
equalizer-import-too-many = 未导入：启用了 { $count ->
        [one] { $count } 个滤波器
       *[other] { $count } 个滤波器
    }，超出配置文件的上限
equalizer-import-too-large = 未导入：文本超过 1 MB，对均衡器配置文件来说太大
equalizer-filter-limit = 一个配置文件最多包含 { $max } 个滤波器。
equalizer-paste-empty = 剪贴板中没有文本
equalizer-copied = 已将配置文件复制为 Equalizer APO 文本
equalizer-exported = 已导出配置文件
equalizer-file-unreadable = 无法读取文件
equalizer-file-unwritable = 无法写入文件
equalizer-value-refused = 已保留之前的值
equalizer-skipped-line = 第 { $line } 行：{ $reason }
equalizer-skipped-more = 以及另外 { $count ->
        [one] { $count } 行
       *[other] { $count } 行
    }
equalizer-skip-unknown-type = 未知的滤波器类型 { $name }
equalizer-skip-unreadable = 不是 Sonora 能读取的滤波器
equalizer-skip-no-frequency = 缺少频率
equalizer-skip-no-gain = 缺少增益
equalizer-skip-no-width = 缺少 Q 值或带宽
equalizer-skip-expression = 数值以表达式形式书写
equalizer-skip-no-preamp = 前级放大不是数字
equalizer-skip-unsupported = 不支持 { $name }
equalizer-rules = 设备规则
equalizer-rule-other = 其他任意设备
equalizer-rule-playing = 正在播放
equalizer-rule-replaces = 替换其规则
equalizer-rule-choose-device = 选择设备
equalizer-rule-choose-sound = 选择音效
equalizer-rule-add = 添加规则
equalizer-rule-remove = 移除规则
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = 频率必须介于 { $low } 到 { $high } Hz 之间
equalizer-invalid-gain = 增益必须介于 { $low } 到 { $high } dB 之间
equalizer-invalid-q = Q 值必须介于 { $low } 到 { $high } 之间
equalizer-invalid-octaves = 带宽必须介于 { $low } 到 { $high } 倍频程之间
equalizer-invalid-shelf-octaves = 架式滤波器使用 Q 值，而非带宽
equalizer-invalid-preamp = 前级放大必须介于 { $low } 到 { $high } dB 之间
equalizer-overload = 削波 { $db } dB
equalizer-overload-slight = 削波不足 0.1 dB
equalizer-lower-preamp = 降低前级放大
equalizer-level = 电平
equalizer-status-bit-perfect = 比特完美
equalizer-status-bit-perfect-detail = Sonora 输出比特完美
equalizer-status-server = 随后声音服务器会将其转换为 { $khz } kHz
equalizer-status-altered = 非比特完美，原因：
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = 音量
equalizer-status-normalized = 响度标准化
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } 位
equalizer-status-float = 浮点 → { $to } 位
equalizer-status-channels = { $from ->
    [1] 单声道
    [2] 立体声
   *[other] { $from } 声道
} → { $to ->
    [1] 单声道
    [2] 立体声
   *[other] { $to } 声道
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB

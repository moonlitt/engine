/**
 * General MIDI Level 1 Program → Chinese display name (128 patches).
 *
 * Used by the channel rows: when a MIDI channel emits a Program Change,
 * we look the program up here and show the Chinese name as the row's
 * subtitle. Channel 10 (display number 10) is GM-reserved for percussion
 * and gets the "鼓组" label regardless of its Program Change value.
 *
 * Indexed by program number 0..127.
 */
export const GM_PROGRAM_ZH: readonly string[] = [
  // Piano (0-7)
  '大钢琴', '亮音大钢琴', '电钢琴', '酒吧钢琴', '电钢琴 1', '电钢琴 2', '羽管键琴', '击弦古钢琴',
  // Chromatic Percussion (8-15)
  '钢片琴', '钟琴', '音乐盒', '颤音琴', '马林巴', '木琴', '管钟', '扬琴',
  // Organ (16-23)
  '击杆风琴', '打击式风琴', '摇滚风琴', '管风琴', '簧风琴', '手风琴', '口琴', '探戈手风琴',
  // Guitar (24-31)
  '尼龙弦吉他', '钢弦吉他', '爵士吉他', '清音电吉他', '弱音电吉他', '过载电吉他', '失真电吉他', '吉他和声',
  // Bass (32-39)
  '原声贝斯', '指弹电贝斯', '拨片电贝斯', '无品贝斯', '击弦贝斯 1', '击弦贝斯 2', '合成贝斯 1', '合成贝斯 2',
  // Strings (40-47)
  '小提琴', '中提琴', '大提琴', '低音提琴', '颤音弦乐', '拨奏弦乐', '竖琴', '定音鼓',
  // Ensemble (48-55)
  '弦乐合奏 1', '弦乐合奏 2', '合成弦乐 1', '合成弦乐 2', '人声啊', '人声哦', '合成人声', '管弦乐齐奏',
  // Brass (56-63)
  '小号', '长号', '大号', '弱音小号', '圆号', '铜管组', '合成铜管 1', '合成铜管 2',
  // Reed (64-71)
  '高音萨克斯', '中音萨克斯', '次中音萨克斯', '上低音萨克斯', '双簧管', '英国管', '巴松管', '单簧管',
  // Pipe (72-79)
  '短笛', '长笛', '竖笛', '排箫', '吹瓶', '尺八', '哨笛', '陶笛',
  // Synth Lead (80-87)
  '方波主音', '锯齿波主音', '汽笛主音', '吹奏主音', '电荷主音', '人声主音', '五度主音', '低音吉他主音',
  // Synth Pad (88-95)
  '空气铺底', '温暖铺底', '复音铺底', '合唱铺底', '弓弦铺底', '金属铺底', '光环铺底', '扫弦铺底',
  // Synth Effects (96-103)
  '雨声效果', '声轨效果', '水晶效果', '气氛效果', '光辉效果', '齐奏效果', '回声效果', '科幻效果',
  // Ethnic (104-111)
  '西塔琴', '班卓琴', '三味线', '古筝', '卡林巴琴', '风笛', '弓弦提琴', '唢呐',
  // Percussive (112-119)
  '叮当铃', '钢鼓', '木鱼', '太鼓', '通通鼓', '合成鼓', '反向钹', '吉他打弦音',
  // Sound Effects (120-127)
  '吉他换弦音', '呼吸声', '海浪声', '鸟鸣声', '电话铃声', '直升机声', '掌声', '枪声',
];

/**
 * Resolve a channel's display label. Priority:
 *   1. MIDI TrackName (already cleaned upstream)
 *   2. Channel 10 → "鼓组" (GM percussion convention)
 *   3. GM Chinese name from the first Program Change
 *   4. Fallback "通道 N"
 */
export function channelDisplayName(
  displayNumber: number,
  trackName: string | undefined,
  program: number | undefined,
): string {
  if (trackName) return trackName;
  if (displayNumber === 10) return '鼓组';
  if (program !== undefined && program >= 0 && program < GM_PROGRAM_ZH.length) {
    return GM_PROGRAM_ZH[program];
  }
  return `通道 ${displayNumber}`;
}

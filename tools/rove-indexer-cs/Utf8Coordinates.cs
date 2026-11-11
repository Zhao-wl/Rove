using Microsoft.CodeAnalysis.Text;

namespace Rove.Indexer;

/// UTF-16 位置 → UTF-8 字节偏移。只记录非 ASCII 字符之后的累计差值，纯 ASCII 文件为空数组。
sealed class Utf8Coordinates
{
    readonly (int utf16, int extra)[] _checkpoints;

    public Utf8Coordinates(SourceText source)
    {
        var text = source.ToString();
        var checkpoints = new List<(int, int)>();
        int extra = 0;
        for (int i = 0; i < text.Length; i++)
        {
            char c = text[i];
            if (c <= 0x7F) continue;
            if (char.IsHighSurrogate(c) && i + 1 < text.Length && char.IsLowSurrogate(text[i + 1]))
            {
                extra += 2;
                i++;
            }
            else extra += c <= 0x7FF ? 1 : 2;
            checkpoints.Add((i + 1, extra));
        }
        _checkpoints = checkpoints.ToArray();
    }

    public uint ByteOffset(int utf16)
    {
        int low = 0, high = _checkpoints.Length;
        while (low < high)
        {
            int mid = low + (high - low) / 2;
            if (_checkpoints[mid].utf16 <= utf16) low = mid + 1;
            else high = mid;
        }
        return (uint)utf16 + (low == 0 ? 0 : (uint)_checkpoints[low - 1].extra);
    }
}

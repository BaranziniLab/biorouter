import { useEffect, useState } from 'react';
import { all_response_styles, ResponseStyleSelectionItem } from './ResponseStyleSelectionItem';

export const ResponseStylesSection = () => {
  const [currentStyle, setCurrentStyle] = useState('concise');

  useEffect(() => {
    const savedStyle = localStorage.getItem('response_style');
    if (savedStyle) {
      try {
        setCurrentStyle(savedStyle);
      } catch (error) {
        console.error('Error parsing response style:', error);
      }
    } else {
      // Set default to concise for new users
      localStorage.setItem('response_style', 'concise');
      setCurrentStyle('concise');
    }
  }, []);

  const handleStyleChange = async (newStyle: string) => {
    setCurrentStyle(newStyle);
    localStorage.setItem('response_style', newStyle);

    // Dispatch custom event to notify other components of the change
    window.dispatchEvent(new CustomEvent('responseStyleChanged'));
  };

  // A fragment: the rows belong directly to the `.biorouter-settings-list` this
  // section mounts into. `space-y-1` could only go once the per-item wrapper in
  // `ResponseStyleSelectionItem` did — before that it was the two rows' only
  // separation, because every row was suppressing its own hairline.
  return (
    <>
      {all_response_styles.map((style) => (
        <ResponseStyleSelectionItem
          key={style.key}
          style={style}
          currentStyle={currentStyle}
          showDescription={true}
          handleStyleChange={handleStyleChange}
        />
      ))}
    </>
  );
};
